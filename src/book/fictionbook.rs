use super::BookNavigation;
use super::dispatch::{BookAsset, BookBackend, BookDescriptor, BookOpenOptions};
use super::xml::parse_xml;
use super::{BookMetadata, TocEntry};
use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use roxmltree::Node;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

struct BinaryResource {
    name: String,
    encoded: String,
}
pub(super) struct FictionBook {
    descriptor: BookDescriptor,
    documents: BTreeMap<String, String>,
    binaries: BTreeMap<String, BinaryResource>,
    resource_bytes_max: u64,
}
struct ContentNode<'a, 'input> {
    node: Node<'a, 'input>,
    content_id: String,
}
struct RenderContext<'a> {
    document: &'a str,
    fragments: &'a BTreeMap<String, String>,
    images: &'a BTreeMap<String, String>,
}

fn fb2(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == name
        && node.tag_name().namespace() == Some("http://www.gribuser.ru/xml/fictionbook/2.0")
}
fn text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn field(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.descendants()
        .find(|node| fb2(*node, name))
        .map(text)
        .filter(|value| !value.is_empty())
}
fn section_id(node: Node<'_, '_>) -> String {
    node.attribute("id")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("fb2-section-{}", node.range().start))
}

fn xml_text(bytes: &[u8]) -> Result<String> {
    let (encoding, skip) = encoding_rs::Encoding::for_bom(bytes).unwrap_or_else(|| {
        let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
        let label = prefix
            .find("encoding")
            .and_then(|index| prefix[index + 8..].trim_start().strip_prefix('='))
            .map(str::trim_start)
            .and_then(|value| {
                let quote = value.chars().next()?;
                if !matches!(quote, '\'' | '"') {
                    return None;
                }
                let label = value.get(1..)?.split(quote).next()?;
                encoding_rs::Encoding::for_label(label.as_bytes())
            });
        (label.unwrap_or(encoding_rs::UTF_8), 0)
    });
    let (text, errors) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if errors {
        return Err(Error::Compatibility(
            "The FB2 character encoding is invalid.".into(),
        ));
    }
    Ok(text.into_owned())
}

impl FictionBook {
    pub fn open(path: &Path, options: &BookOpenOptions) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(options.limits.archive_bytes_max.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > options.limits.archive_bytes_max {
            return Err(Error::Compatibility(
                "The FB2 exceeds its file size limit.".into(),
            ));
        }
        options.cancellation.check()?;
        let xml = xml_text(&bytes)?;
        let document = parse_xml(&xml)?;
        let root = document.root_element();
        if !fb2(root, "FictionBook") {
            return Err(Error::Compatibility(
                "The file is not a FictionBook document.".into(),
            ));
        }
        let authors = root
            .descendants()
            .find(|node| fb2(*node, "title-info"))
            .into_iter()
            .flat_map(|node| node.children())
            .filter(|node| fb2(*node, "author"))
            .map(|author| {
                ["first-name", "middle-name", "last-name"]
                    .into_iter()
                    .filter_map(|name| field(author, name))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|author| !author.is_empty())
            .collect::<Vec<_>>();
        let metadata = BookMetadata {
            title: field(root, "book-title"),
            creator: (!authors.is_empty()).then(|| authors.join(", ")),
            description: field(root, "annotation"),
            publisher: field(root, "publisher"),
            date: field(root, "date"),
            language: field(root, "lang"),
            identifier: field(root, "id"),
            source: field(root, "src-url"),
            format: Some("FB2".into()),
        };
        let mut nodes = Vec::new();
        for (body_index, body) in root
            .children()
            .filter(|node| fb2(*node, "body"))
            .enumerate()
        {
            for (index, node) in body.children().filter(Node::is_element).enumerate() {
                nodes.push(ContentNode {
                    node,
                    content_id: format!("body-{body_index:04}/content-{index:04}.xhtml"),
                });
            }
        }
        if nodes.len() > options.limits.archive_entries_max {
            return Err(Error::Compatibility(
                "The FB2 exceeds its document count limit.".into(),
            ));
        }
        let mut fragments = BTreeMap::new();
        for content in &nodes {
            for node in content.node.descendants().filter(Node::is_element) {
                if let Some(id) = node.attribute("id") {
                    fragments
                        .entry(id.into())
                        .or_insert_with(|| content.content_id.clone());
                }
            }
        }
        let mut binaries = BTreeMap::new();
        let mut images = BTreeMap::new();
        for (index, node) in root
            .children()
            .filter(|node| fb2(*node, "binary"))
            .enumerate()
        {
            let Some(id) = node.attribute("id") else {
                continue;
            };
            let extension = match node.attribute("content-type").unwrap_or_default() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                "image/gif" => "gif",
                "image/webp" => "webp",
                "image/svg+xml" => "svg",
                _ => "bin",
            };
            let name = format!("images/image-{index:04}.{extension}");
            let encoded = node
                .text()
                .unwrap_or_default()
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            if images.insert(id.into(), name.clone()).is_some() {
                return Err(Error::Compatibility(
                    "The FB2 contains duplicate image identifiers.".into(),
                ));
            }
            binaries.insert(name.clone(), BinaryResource { name, encoded });
        }
        let mut documents = BTreeMap::new();
        let mut toc = Vec::new();
        for (index, content) in nodes.iter().enumerate() {
            options.cancellation.check()?;
            let mut markup = String::new();
            render_node(
                content.node,
                &RenderContext {
                    document: &content.content_id,
                    fragments: &fragments,
                    images: &images,
                },
                &mut markup,
                0,
            )?;
            if markup.len() as u64 > options.limits.resource_bytes_max {
                return Err(Error::Compatibility(
                    "The FB2 section exceeds its resource size limit.".into(),
                ));
            }
            for section in content
                .node
                .descendants()
                .filter(|node| fb2(*node, "section"))
            {
                if let Some(title) = section.children().find(|node| fb2(*node, "title")) {
                    toc.push(TocEntry {
                        label: text(title),
                        content_index: index,
                        section: Some(
                            percent_encoding::utf8_percent_encode(
                                &section_id(section),
                                percent_encoding::NON_ALPHANUMERIC,
                            )
                            .to_string(),
                        ),
                    });
                }
            }
            documents.insert(content.content_id.clone(), markup);
        }
        let contents = nodes
            .iter()
            .map(|node| node.content_id.clone())
            .collect::<Vec<_>>();
        let spine_len = contents.len();
        Ok(Self {
            descriptor: BookDescriptor {
                metadata,
                contents,
                spine_len,
                toc,
                navigation: BookNavigation::default(),
            },
            documents,
            binaries,
            resource_bytes_max: options.limits.resource_bytes_max,
        })
    }
}

fn relative_target(from: &str, target: &str) -> String {
    let mut relative = "../".repeat(from.split('/').count().saturating_sub(1));
    relative.push_str(target);
    relative
}

fn render_node(
    node: Node<'_, '_>,
    context: &RenderContext<'_>,
    output: &mut String,
    depth: usize,
) -> Result<()> {
    if depth > 256 {
        return Err(Error::Compatibility(
            "The FB2 section exceeds its nesting limit.".into(),
        ));
    }
    if node.is_text() {
        output.push_str(&html_escape::encode_text(node.text().unwrap_or_default()));
        return Ok(());
    }
    if !node.is_element() {
        return Ok(());
    }
    let local = node.tag_name().name();
    let tag = match local {
        "title" => "h1",
        "subtitle" => "h2",
        "section" | "stanza" => "div",
        "emphasis" => "em",
        "strikethrough" => "s",
        "poem" | "epigraph" => "blockquote",
        "v" => "p",
        "image" => "img",
        "empty-line" => "br",
        _ => local,
    };
    output.push('<');
    output.push_str(tag);
    let id = if local == "section" {
        Some(section_id(node))
    } else {
        node.attribute("id").map(str::to_owned)
    };
    if let Some(id) = id {
        output.push_str(" id=\"");
        output.push_str(&html_escape::encode_double_quoted_attribute(&id));
        output.push('"');
    }
    if let Some(href) = node
        .attribute(("http://www.w3.org/1999/xlink", "href"))
        .or_else(|| node.attribute("href"))
    {
        let href = if let Some(id) = href.strip_prefix('#') {
            let id = percent_encoding::percent_decode_str(id).decode_utf8_lossy();
            if local == "image" {
                context
                    .images
                    .get(id.as_ref())
                    .map(|path| relative_target(context.document, path))
                    .unwrap_or_else(|| href.into())
            } else {
                context
                    .fragments
                    .get(id.as_ref())
                    .map(|path| {
                        format!(
                            "{}#{}",
                            relative_target(context.document, path),
                            percent_encoding::utf8_percent_encode(
                                &id,
                                percent_encoding::NON_ALPHANUMERIC
                            )
                        )
                    })
                    .unwrap_or_else(|| href.into())
            }
        } else {
            href.into()
        };
        output.push_str(if local == "image" {
            " src=\""
        } else {
            " href=\""
        });
        output.push_str(&html_escape::encode_double_quoted_attribute(&href));
        output.push('"');
    }
    if local == "a" && node.attribute("type") == Some("note") {
        output.push_str(" role=\"doc-noteref\"");
    }
    if matches!(tag, "br" | "img") {
        output.push_str("/>");
        return Ok(());
    }
    output.push('>');
    for child in node.children() {
        render_node(child, context, output, depth + 1)?;
    }
    output.push_str("</");
    output.push_str(tag);
    output.push('>');
    Ok(())
}

impl BookBackend for FictionBook {
    fn descriptor(&self) -> &BookDescriptor {
        &self.descriptor
    }
    fn raw_text(&mut self, id: &str) -> Result<String> {
        self.documents
            .get(id)
            .cloned()
            .ok_or_else(|| Error::Compatibility("The FB2 document does not exist.".into()))
    }
    fn asset(&mut self, id: &str) -> Result<BookAsset> {
        let resource = self
            .binaries
            .get(id)
            .ok_or_else(|| Error::Compatibility("The FB2 image does not exist.".into()))?;
        if resource.encoded.len() as u64 > self.resource_bytes_max.saturating_mul(4) / 3 + 4 {
            return Err(Error::Compatibility(
                "The FB2 image exceeds its size limit.".into(),
            ));
        }
        let bytes = STANDARD
            .decode(&resource.encoded)
            .map_err(|error| Error::Compatibility(format!("Invalid FB2 image: {error}")))?;
        Ok(BookAsset {
            name: resource.name.clone(),
            bytes,
        })
    }
}
