use super::BookNavigation;
use super::{BookLocation, BookMetadata, ResourceLimits, TocEntry};
use crate::cancellation::Cancellation;
use crate::{Error, Result};
use percent_encoding::percent_decode_str;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookKind {
    Epub,
    FictionBook,
    Kindle,
    Web,
}

#[derive(Default)]
pub struct BookOpenOptions {
    pub limits: ResourceLimits,
    pub cancellation: Cancellation,
}

pub struct BookAsset {
    pub name: String,
    pub bytes: Vec<u8>,
}
impl BookAsset {
    pub(crate) fn new(resource: &str, bytes: Vec<u8>) -> Self {
        Self {
            name: resource.rsplit('/').next().unwrap_or("image").into(),
            bytes,
        }
    }
}

pub struct ResourceTarget {
    pub content_id: String,
    pub fragment: Option<String>,
}

pub(crate) struct BookDescriptor {
    pub metadata: BookMetadata,
    pub contents: Vec<String>,
    pub spine_len: usize,
    pub toc: Vec<TocEntry>,
    pub navigation: BookNavigation,
}

pub(crate) trait BookBackend: Send {
    fn descriptor(&self) -> &BookDescriptor;
    fn raw_text(&mut self, content_id: &str) -> Result<String>;
    fn asset(&mut self, resource_id: &str) -> Result<BookAsset>;
    fn resolve_resource(&self, from: &str, href: &str) -> Result<ResourceTarget> {
        let (path, fragment) = href
            .split_once('#')
            .map_or((href, None), |(path, fragment)| (path, Some(fragment)));
        let path =
            percent_decode_str(path.split('?').next().unwrap_or_default()).decode_utf8_lossy();
        let content_id = if path.is_empty() {
            from.into()
        } else {
            super::epub::join_resource(from, &path)?
        };
        Ok(ResourceTarget {
            content_id,
            fragment: fragment
                .filter(|value| !value.is_empty())
                .map(|value| percent_decode_str(value).decode_utf8_lossy().into_owned()),
        })
    }
}

pub struct Book {
    backend: Box<dyn BookBackend>,
    location: BookLocation,
    kind: BookKind,
    options: BookOpenOptions,
}

impl Book {
    pub fn open(location: impl Into<BookLocation>) -> Result<Self> {
        Self::open_with_options(location.into(), BookOpenOptions::default())
    }
    pub fn open_with_options(location: BookLocation, options: BookOpenOptions) -> Result<Self> {
        options.cancellation.check()?;
        let (backend, location, kind) = match location {
            BookLocation::Local(path) => {
                let path = path.canonicalize()?;
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let kind = match extension.as_str() {
                    "epub" | "epub3" => BookKind::Epub,
                    "fb2" => BookKind::FictionBook,
                    "mobi" | "azw" | "azw3" => BookKind::Kindle,
                    _ => {
                        return Err(Error::Compatibility(format!(
                            "Unsupported ebook format: {extension}"
                        )));
                    }
                };
                let backend = open_local(&path, kind, &options)?;
                (backend, BookLocation::Local(path), kind)
            }
            BookLocation::Web(url) => {
                let backend =
                    Box::new(super::web::WebBook::open(&url, &options)?) as Box<dyn BookBackend>;
                (backend, BookLocation::Web(url), BookKind::Web)
            }
        };
        if backend.descriptor().contents.is_empty() {
            return Err(Error::Compatibility(
                "The ebook contains no readable documents.".into(),
            ));
        }
        Ok(Self {
            backend,
            location,
            kind,
            options,
        })
    }
    pub fn metadata(&self) -> &BookMetadata {
        &self.backend.descriptor().metadata
    }
    pub fn contents(&self) -> &[String] {
        &self.backend.descriptor().contents
    }
    pub fn spine_contents(&self) -> &[String] {
        &self.contents()[..self.backend.descriptor().spine_len]
    }
    pub fn toc(&self) -> &[TocEntry] {
        &self.backend.descriptor().toc
    }
    pub fn navigation(&self) -> &BookNavigation {
        &self.backend.descriptor().navigation
    }
    pub fn kind(&self) -> BookKind {
        self.kind
    }
    pub fn annotations_enabled(&self) -> bool {
        self.kind == BookKind::Epub
    }
    pub fn location(&self) -> &BookLocation {
        &self.location
    }
    pub fn raw_text(&mut self, content_id: &str) -> Result<String> {
        self.options.cancellation.check()?;
        let text = self.backend.raw_text(content_id)?;
        if text.len() as u64 > self.options.limits.resource_bytes_max {
            return Err(Error::Compatibility(
                "The ebook document exceeds its size limit.".into(),
            ));
        }
        Ok(text)
    }
    pub fn asset(&mut self, resource_id: &str) -> Result<BookAsset> {
        self.options.cancellation.check()?;
        let asset = self.backend.asset(resource_id)?;
        if asset.bytes.len() as u64 > self.options.limits.resource_bytes_max {
            return Err(Error::Compatibility(
                "The ebook asset exceeds its size limit.".into(),
            ));
        }
        Ok(asset)
    }
    pub fn resolve_resource(&self, from: &str, href: &str) -> Result<ResourceTarget> {
        self.backend.resolve_resource(from, href)
    }
}

fn open_local(
    path: &Path,
    kind: BookKind,
    options: &BookOpenOptions,
) -> Result<Box<dyn BookBackend>> {
    match kind {
        BookKind::Epub => Ok(Box::new(super::epub::EpubBook::open_with_limits(
            path,
            options.limits,
        )?)),
        BookKind::FictionBook => Ok(Box::new(super::fictionbook::FictionBook::open(
            path, options,
        )?)),
        BookKind::Kindle => Ok(Box::new(super::kindle::KindleBook::open(path, options)?)),
        BookKind::Web => Err(Error::Compatibility("Use a URL for web content.".into())),
    }
}
