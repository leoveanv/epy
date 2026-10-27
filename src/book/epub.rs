use std::fs::File;
use std::io::Read;
use std::path::Path;

use super::xml::parse_xml;
use percent_encoding::percent_decode_str;
use roxmltree::{Document, Node};
use zip::ZipArchive;

use super::{BookMetadata, ResourceLimits, TocEntry};
use super::{BookNavigation, parse_navigation};
use crate::{Error, Result};

pub struct EpubBook {
    archive: ZipArchive<File>,
    descriptor: super::dispatch::BookDescriptor,
    limits: ResourceLimits,
}

#[derive(Clone, Copy)]
enum NavigationKind {
    Html,
    Ncx,
}

struct NavigationResource {
    path: String,
    kind: NavigationKind,
}

struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
    properties: String,
}

struct Package {
    version: String,
    metadata: BookMetadata,
    manifest: Vec<ManifestItem>,
    spine: Vec<String>,
}

impl EpubBook {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_limits(path, ResourceLimits::default())
    }

    pub fn open_with_limits(path: &Path, limits: ResourceLimits) -> Result<Self> {
        let mut archive = ZipArchive::new(File::open(path)?)?;
        validate_archive(&mut archive, limits)?;
        let container_xml = archive_text(&mut archive, "META-INF/container.xml", limits)?;
        let package_path = package_path(&container_xml)?;
        let package_xml = archive_text(&mut archive, &package_path, limits)?;
        let package = parse_package(&package_xml)?;
        let relative_contents = spine_contents(&package);
        let mut contents = relative_contents
            .iter()
            .map(|href| join_resource(&package_path, href))
            .collect::<Result<Vec<_>>>()?;
        let resource = navigation_resource(&package, &package_path)?;
        let toc_xml = archive_text(&mut archive, &resource.path, limits)?;
        let toc = parse_toc(&toc_xml, &resource, &contents)?;
        let navigation = parse_navigation(&toc_xml, &resource.path)?;
        let spine_len = contents.len();
        append_supplemental(&mut contents, &package.manifest, &package_path)?;
        Ok(Self {
            archive,
            descriptor: super::dispatch::BookDescriptor {
                metadata: package.metadata,
                contents,
                spine_len,
                toc,
                navigation,
            },
            limits,
        })
    }

    pub fn metadata(&self) -> &BookMetadata {
        &self.descriptor.metadata
    }

    pub fn contents(&self) -> &[String] {
        &self.descriptor.contents
    }

    pub fn spine_contents(&self) -> &[String] {
        &self.descriptor.contents[..self.descriptor.spine_len]
    }

    pub fn toc(&self) -> &[TocEntry] {
        &self.descriptor.toc
    }

    pub fn navigation(&self) -> &BookNavigation {
        &self.descriptor.navigation
    }

    pub fn raw_text(&mut self, content_id: &str) -> Result<String> {
        archive_text(&mut self.archive, content_id, self.limits)
    }

    pub fn asset(&mut self, resource_id: &str) -> Result<Vec<u8>> {
        archive_bytes(&mut self.archive, resource_id, self.limits)
    }
}

impl super::dispatch::BookBackend for EpubBook {
    fn descriptor(&self) -> &super::dispatch::BookDescriptor {
        &self.descriptor
    }
    fn raw_text(&mut self, id: &str) -> Result<String> {
        EpubBook::raw_text(self, id)
    }
    fn asset(&mut self, id: &str) -> Result<super::dispatch::BookAsset> {
        Ok(super::dispatch::BookAsset::new(
            id,
            EpubBook::asset(self, id)?,
        ))
    }
}

fn validate_archive(archive: &mut ZipArchive<File>, limits: ResourceLimits) -> Result<()> {
    if archive.len() > limits.archive_entries_max {
        return Err(Error::Compatibility(
            "The ebook exceeds the archive entry limit.".into(),
        ));
    }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let resource = archive.by_index(index)?;
        total = total.checked_add(resource.size()).ok_or_else(|| {
            Error::Compatibility("The ebook's declared resource sizes overflow.".into())
        })?;
        if total > limits.archive_bytes_max {
            return Err(Error::Compatibility(
                "The ebook exceeds the uncompressed archive size limit.".into(),
            ));
        }
    }
    Ok(())
}

fn validate_resource_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains(['\0', '\\'])
        || name.split('/').any(|segment| segment == "..")
    {
        return Err(Error::Compatibility(
            "The ebook resource path is unsafe.".into(),
        ));
    }
    Ok(())
}

fn archive_bytes(
    archive: &mut ZipArchive<File>,
    name: &str,
    limits: ResourceLimits,
) -> Result<Vec<u8>> {
    validate_resource_name(name)?;
    let resource = archive.by_name(name)?;
    if resource.size() > limits.resource_bytes_max {
        return Err(Error::Compatibility(
            "The ebook resource exceeds the size limit.".into(),
        ));
    }
    let mut bytes = Vec::new();
    resource
        .take(limits.resource_bytes_max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limits.resource_bytes_max {
        return Err(Error::Compatibility(
            "The decompressed ebook resource exceeds the size limit.".into(),
        ));
    }
    Ok(bytes)
}

fn archive_text(
    archive: &mut ZipArchive<File>,
    name: &str,
    limits: ResourceLimits,
) -> Result<String> {
    String::from_utf8(archive_bytes(archive, name, limits)?)
        .map_err(|error| Error::Compatibility(format!("The ebook resource is not UTF-8: {error}")))
}

fn namespace(node: Node<'_, '_>, namespace: &str, local: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(namespace)
        && node.tag_name().name() == local
}

fn required_attribute(node: Node<'_, '_>, name: &str) -> Result<String> {
    node.attribute(name).map(str::to_owned).ok_or_else(|| {
        Error::Compatibility(format!(
            "The ebook's {} element is missing {name}.",
            node.tag_name().name()
        ))
    })
}

fn package_path(container_xml: &str) -> Result<String> {
    let container = parse_xml(container_xml)?;
    let rootfile = container
        .descendants()
        .find(|node| {
            namespace(
                *node,
                "urn:oasis:names:tc:opendocument:xmlns:container",
                "rootfile",
            )
        })
        .ok_or_else(|| Error::Compatibility("The EPUB has no package document.".into()))?;
    let path = required_attribute(rootfile, "full-path")?;
    validate_resource_name(&path)?;
    Ok(path)
}

fn metadata_field(package: &Document<'_>, name: &str) -> Option<String> {
    package
        .descendants()
        .find(|node| namespace(*node, "http://purl.org/dc/elements/1.1/", name))
        .and_then(|node| node.text())
        .map(str::to_owned)
}

fn parse_package(xml: &str) -> Result<Package> {
    let document = parse_xml(xml)?;
    let version = required_attribute(document.root_element(), "version")?;
    if !matches!(version.as_str(), "1.0" | "2.0" | "3.0") {
        return Err(Error::Compatibility(format!(
            "Unsupported Epub version: {version}"
        )));
    }
    let metadata = BookMetadata {
        title: metadata_field(&document, "title"),
        creator: metadata_field(&document, "creator"),
        description: metadata_field(&document, "description"),
        publisher: metadata_field(&document, "publisher"),
        date: metadata_field(&document, "date"),
        language: metadata_field(&document, "language"),
        format: metadata_field(&document, "format"),
        identifier: metadata_field(&document, "identifier"),
        source: metadata_field(&document, "source"),
    };
    let manifest = document
        .root_element()
        .children()
        .find(|node| namespace(*node, "http://www.idpf.org/2007/opf", "manifest"))
        .ok_or_else(|| Error::Compatibility("The EPUB has no manifest.".into()))?;
    let manifest = manifest
        .children()
        .filter(Node::is_element)
        .map(|node| {
            Ok(ManifestItem {
                id: required_attribute(node, "id")?,
                href: required_attribute(node, "href")?,
                media_type: node.attribute("media-type").unwrap_or_default().into(),
                properties: node.attribute("properties").unwrap_or_default().into(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let spine = document
        .root_element()
        .children()
        .find(|node| namespace(*node, "http://www.idpf.org/2007/opf", "spine"))
        .ok_or_else(|| Error::Compatibility("The EPUB has no spine.".into()))?;
    let spine = spine
        .children()
        .filter(Node::is_element)
        .map(|node| required_attribute(node, "idref"))
        .collect::<Result<Vec<_>>>()?;
    Ok(Package {
        version,
        metadata,
        manifest,
        spine,
    })
}

fn spine_contents(package: &Package) -> Vec<String> {
    let mut remaining = package
        .manifest
        .iter()
        .filter(|item| {
            item.media_type != "application/x-dtbncx+xml"
                && !item
                    .properties
                    .split_whitespace()
                    .any(|property| property == "nav")
        })
        .collect::<Vec<_>>();
    let mut contents = Vec::new();
    for id in &package.spine {
        if let Some(index) = remaining.iter().position(|item| item.id == *id) {
            contents.push(
                percent_decode_str(&remaining.remove(index).href)
                    .decode_utf8_lossy()
                    .into_owned(),
            );
        }
    }
    contents
}

fn navigation_resource(package: &Package, package_path: &str) -> Result<NavigationResource> {
    let html = package.manifest.iter().find(|item| {
        item.properties
            .split_whitespace()
            .any(|property| property == "nav")
    });
    let ncx = package
        .manifest
        .iter()
        .find(|item| item.media_type == "application/x-dtbncx+xml");
    let item = if package.version == "3.0" {
        html.or(ncx)
    } else {
        ncx.or(html)
    }
    .ok_or_else(|| Error::Compatibility("The EPUB has no supported navigation document.".into()))?;
    let path = join_resource(
        package_path,
        &percent_decode_str(&item.href).decode_utf8_lossy(),
    )?;
    let kind = if item.media_type == "application/x-dtbncx+xml" {
        NavigationKind::Ncx
    } else {
        NavigationKind::Html
    };
    Ok(NavigationResource { path, kind })
}

fn append_supplemental(
    contents: &mut Vec<String>,
    manifest: &[ManifestItem],
    package_path: &str,
) -> Result<()> {
    for item in manifest {
        if matches!(
            item.media_type.as_str(),
            "application/xhtml+xml" | "text/html"
        ) && !item
            .properties
            .split_whitespace()
            .any(|property| property == "nav")
        {
            let path = percent_decode_str(&join_resource(package_path, &item.href)?)
                .decode_utf8_lossy()
                .into_owned();
            if !contents.contains(&path) {
                contents.push(path);
            }
        }
    }
    Ok(())
}

pub(crate) fn join_resource(base_document: &str, reference: &str) -> Result<String> {
    if reference.starts_with('/')
        || reference
            .split('/')
            .next()
            .is_some_and(|first| first.contains(':'))
    {
        return Err(Error::Compatibility(
            "An ebook resource cannot name an external or absolute path.".into(),
        ));
    }
    let base = base_document
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let mut segments = base
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    for segment in reference.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(Error::Compatibility(
                        "An ebook resource escapes its archive root.".into(),
                    ));
                }
            }
            _ => segments.push(segment),
        }
    }
    let path = segments.join("/");
    validate_resource_name(&path)?;
    Ok(path)
}

fn parse_toc(
    xml: &str,
    resource: &NavigationResource,
    contents: &[String],
) -> Result<Vec<TocEntry>> {
    let document = parse_xml(xml)?;
    let mut entries = Vec::new();
    match resource.kind {
        NavigationKind::Html => {
            for node in document
                .descendants()
                .filter(|node| namespace(*node, "http://www.w3.org/1999/xhtml", "a"))
            {
                if !node.ancestors().any(|ancestor| {
                    namespace(ancestor, "http://www.w3.org/1999/xhtml", "nav")
                        && ancestor
                            .attribute(("http://www.idpf.org/2007/ops", "type"))
                            .is_some_and(|types| types.split_whitespace().any(|kind| kind == "toc"))
                }) {
                    continue;
                }
                let target = required_attribute(node, "href")?;
                let label = node
                    .descendants()
                    .filter(Node::is_text)
                    .filter_map(|node| node.text())
                    .collect::<String>();
                if let Some(entry) = toc_entry(
                    label,
                    TocTarget {
                        href: &target,
                        base: &resource.path,
                        contents,
                    },
                )? {
                    entries.push(entry);
                }
            }
        }
        NavigationKind::Ncx => {
            for point in document
                .descendants()
                .filter(|node| namespace(*node, "http://www.daisy.org/z3986/2005/ncx/", "navPoint"))
            {
                let target = point
                    .children()
                    .find(|node| {
                        namespace(*node, "http://www.daisy.org/z3986/2005/ncx/", "content")
                    })
                    .ok_or_else(|| {
                        Error::Compatibility("An EPUB navigation point has no content.".into())
                    })?;
                let label = point
                    .children()
                    .find(|node| {
                        namespace(*node, "http://www.daisy.org/z3986/2005/ncx/", "navLabel")
                    })
                    .and_then(|node| {
                        node.children().find(|node| {
                            namespace(*node, "http://www.daisy.org/z3986/2005/ncx/", "text")
                        })
                    })
                    .and_then(|node| node.text());
                if let Some(label) = label
                    && let Some(entry) = toc_entry(
                        label.into(),
                        TocTarget {
                            href: &required_attribute(target, "src")?,
                            base: &resource.path,
                            contents,
                        },
                    )?
                {
                    entries.push(entry);
                }
            }
        }
    }
    Ok(entries)
}

struct TocTarget<'a> {
    href: &'a str,
    base: &'a str,
    contents: &'a [String],
}

fn toc_entry(label: String, target: TocTarget<'_>) -> Result<Option<TocEntry>> {
    let (path, fragment) = target
        .href
        .split_once('#')
        .map_or((target.href, None), |(path, fragment)| {
            (path, Some(fragment))
        });
    if path.starts_with('/')
        || path
            .split('/')
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return Ok(None);
    }
    let path = percent_decode_str(path.split('?').next().unwrap_or_default()).decode_utf8_lossy();
    let path = if path.is_empty() {
        target.base.into()
    } else {
        join_resource(target.base, &path)?
    };
    Ok(target
        .contents
        .iter()
        .position(|content| *content == path)
        .map(|content_index| TocEntry {
            label,
            content_index,
            section: fragment.map(str::to_owned),
        }))
}
