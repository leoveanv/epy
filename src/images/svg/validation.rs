use super::SvgLimits;
use super::styles::{StyleReferences, url_references};
use crate::cancellation::Cancellation;
use crate::{Error, Result};
use roxmltree::{Document, Node};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct SvgSource<'a> {
    pub text: &'a str,
    pub needs_fonts: bool,
    pub image_refs: BTreeSet<String>,
}

pub(super) fn validate_svg<'a>(
    bytes: &'a [u8],
    limits: &SvgLimits,
    cancellation: &Cancellation,
) -> Result<SvgSource<'a>> {
    cancellation.check()?;
    if bytes.len() > limits.encoded_bytes_max {
        return Err(svg_error("source size limit exceeded"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| svg_error("the document is not UTF-8"))?;
    let document = Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: limits.nodes_max as u32,
            ..roxmltree::ParsingOptions::default()
        },
    )
    .map_err(|error| svg_error(&error.to_string()))?;
    if document.root_element().tag_name().name() != "svg" {
        return Err(svg_error("the document has no SVG root"));
    }
    let mut ids = BTreeMap::new();
    for node in document.descendants().filter(|node| node.is_element()) {
        if let Some(id) = node.attribute("id")
            && ids.insert(id, node).is_some()
        {
            return Err(svg_error("duplicate element IDs"));
        }
    }
    let mut validator = Validator {
        ids,
        styles: StyleReferences::parse(&document, limits, cancellation)?,
        limits,
        cancellation,
        nodes: 0,
        text_characters: 0,
        path_bytes: 0,
        image_refs: BTreeSet::new(),
    };
    validator.visit(document.root_element(), 0)?;
    Ok(SvgSource {
        text,
        needs_fonts: document
            .descendants()
            .any(|node| node.is_element() && node.tag_name().name() == "text"),
        image_refs: validator.image_refs,
    })
}

struct Validator<'a, 'input, 'options> {
    ids: BTreeMap<&'a str, Node<'a, 'input>>,
    styles: StyleReferences<'a>,
    limits: &'options SvgLimits,
    cancellation: &'options Cancellation,
    nodes: usize,
    text_characters: usize,
    path_bytes: usize,
    image_refs: BTreeSet<String>,
}

impl<'a, 'input> Validator<'a, 'input, '_> {
    fn visit(&mut self, node: Node<'a, 'input>, depth: usize) -> Result<()> {
        self.cancellation.check()?;
        if depth > self.limits.depth_max {
            return Err(svg_error("depth or reference-cycle limit exceeded"));
        }
        self.nodes += 1;
        if self.nodes > self.limits.nodes_max {
            return Err(svg_error("expanded node limit exceeded"));
        }
        self.validate_node(node)?;
        let mut references = local_references(node);
        references.extend(self.styles.get(node.id()));
        for id in references {
            if let Some(target) = self.ids.get(id).copied() {
                self.visit(target, depth + 1)?;
            }
        }
        for child in node.children() {
            if child.is_element() {
                self.visit(child, depth + 1)?;
            } else if child.is_text()
                && node
                    .ancestors()
                    .any(|ancestor| ancestor.is_element() && ancestor.tag_name().name() == "text")
            {
                self.text_characters = self
                    .text_characters
                    .saturating_add(child.text().unwrap_or_default().chars().count());
                if self.text_characters > self.limits.text_characters_max {
                    return Err(svg_error("text complexity limit exceeded"));
                }
            }
        }
        Ok(())
    }

    fn validate_node(&mut self, node: Node<'a, 'input>) -> Result<()> {
        if matches!(node.tag_name().name(), "image" | "feImage")
            && let Some(href) = href(node)
            && !href.starts_with("data:")
        {
            if href.starts_with(['/', '\\'])
                || url::Url::parse(href).is_ok()
                || href.chars().any(char::is_control)
            {
                return Err(Error::Compatibility(
                    "External SVG image references are disabled.".into(),
                ));
            }
            self.image_refs.insert(href.into());
            if self.image_refs.len() > self.limits.resources_max {
                return Err(svg_error("image resource count limit exceeded"));
            }
        }
        if node.tag_name().name() == "use"
            && href(node).is_some_and(|value| !value.starts_with('#'))
        {
            return Err(Error::Compatibility(
                "External SVG use references are disabled.".into(),
            ));
        }
        if node.tag_name().name() == "path" {
            self.path_bytes = self
                .path_bytes
                .saturating_add(node.attribute("d").unwrap_or_default().len());
            if self.path_bytes > self.limits.path_bytes_max {
                return Err(svg_error("path complexity limit exceeded"));
            }
        }
        if node.tag_name().name() == "feTurbulence"
            && node
                .attribute("numOctaves")
                .and_then(|value| value.parse::<u32>().ok())
                .is_some_and(|value| value > 8)
        {
            return Err(svg_error("turbulence complexity limit exceeded"));
        }
        if node.tag_name().name() == "feConvolveMatrix"
            && node.attribute("order").is_some_and(|value| {
                value
                    .split([' ', ','])
                    .filter_map(|value| value.parse::<u32>().ok())
                    .any(|value| value > 8)
            })
        {
            return Err(svg_error("convolution complexity limit exceeded"));
        }
        Ok(())
    }
}

fn local_references<'a, 'input>(node: Node<'a, 'input>) -> BTreeSet<&'a str> {
    let mut references = node
        .attributes()
        .flat_map(|attribute| url_references(attribute.value()))
        .collect::<BTreeSet<_>>();
    if !matches!(node.tag_name().name(), "a" | "image" | "feImage")
        && let Some(id) = href(node).and_then(|value| value.strip_prefix('#'))
    {
        references.insert(id);
    }
    references
}

fn href<'a, 'input>(node: Node<'a, 'input>) -> Option<&'a str> {
    node.attribute("href")
        .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")))
}

pub(super) fn svg_error(detail: &str) -> Error {
    Error::Compatibility(format!("Cannot preview this SVG image: {detail}"))
}
