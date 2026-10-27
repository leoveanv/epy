mod compression;
mod container;
mod extended;
mod flows;
mod header;
mod huff_decoder;
mod huffman;
mod metadata;
mod prepared;
mod resources;
mod source;

use super::TocEntry;
use super::dispatch::{BookAsset, BookBackend, BookDescriptor, BookOpenOptions};
use crate::book::BookNavigation;
use crate::{Error, Result};
use boko::Importer;
use boko::import::{Azw3Importer, MobiImporter};
use container::KindleContainer;
use source::{LimitedSource, RecordSource};
use std::path::Path;

pub(super) struct KindleBook {
    descriptor: BookDescriptor,
    backend: Box<dyn Importer>,
    container: KindleContainer,
}

fn kindle_error(error: boko::Error) -> Error {
    Error::Compatibility(format!("Cannot decode this Kindle book: {error}"))
}

fn number(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| Error::Compatibility("The Kindle header offset overflows.".into()))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| Error::Compatibility("The Kindle header is truncated.".into()))?;
    Ok(u32::from_be_bytes(value.try_into().map_err(|_| {
        Error::Compatibility("The Kindle header is invalid.".into())
    })?))
}

fn importer(source: std::sync::Arc<dyn boko::ByteSource>) -> Result<Box<dyn Importer>> {
    match Azw3Importer::from_source(source.clone()) {
        Ok(importer) => Ok(Box::new(importer)),
        Err(boko::Error::UnsupportedFormat { detail }) if detail.starts_with("not a KF8") => Ok(
            Box::new(MobiImporter::from_source(source).map_err(kindle_error)?),
        ),
        Err(error) => Err(kindle_error(error)),
    }
}

impl KindleBook {
    pub fn open(path: &Path, options: &BookOpenOptions) -> Result<Self> {
        let source = LimitedSource::open(path, options.limits, options.cancellation.clone())?;
        let records = RecordSource::open(source.clone(), options.limits)?;
        let container = KindleContainer::open(records, options.limits)?;
        let backend = if container.header.mobi_version == 8 {
            importer(prepared::prepare_kf8(
                &container,
                options.cancellation.clone(),
            )?)?
        } else {
            importer(source)?
        };
        let mut metadata = container.metadata.clone();
        metadata.format = Some("Kindle".into());
        let contents = backend
            .spine()
            .iter()
            .map(|entry| {
                backend
                    .source_id(entry.id)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        Error::Compatibility("A Kindle chapter has no source identifier.".into())
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut toc = Vec::new();
        let resolved = backend.resolve_toc();
        flatten_toc(
            resolved.as_deref().unwrap_or_else(|| backend.toc()),
            &contents,
            &mut toc,
            0,
        )?;
        if container.header.mobi_version < 8
            && toc.len() == 1
            && toc[0].label == backend.metadata().title
            && let Some(title) = &metadata.title
        {
            toc[0].label = title.clone();
        }
        let spine_len = contents.len();
        Ok(Self {
            descriptor: BookDescriptor {
                metadata,
                contents,
                spine_len,
                toc,
                navigation: BookNavigation::default(),
            },
            backend,
            container,
        })
    }
}

fn flatten_toc(
    entries: &[boko::TocEntry],
    contents: &[String],
    output: &mut Vec<TocEntry>,
    depth: usize,
) -> Result<()> {
    if depth > 64 || output.len() > 100_000 {
        return Err(Error::Compatibility(
            "The Kindle navigation exceeds its limits.".into(),
        ));
    }
    for entry in entries {
        let (path, fragment) = entry
            .href
            .split_once('#')
            .map_or((entry.href.as_str(), None), |(path, fragment)| {
                (path, Some(fragment))
            });
        if let Some(content_index) = contents.iter().position(|content| content == path) {
            output.push(TocEntry {
                label: entry.title.clone(),
                content_index,
                section: fragment.map(str::to_owned),
            });
        }
        flatten_toc(&entry.children, contents, output, depth + 1)?;
    }
    Ok(())
}

impl BookBackend for KindleBook {
    fn descriptor(&self) -> &BookDescriptor {
        &self.descriptor
    }

    fn raw_text(&mut self, id: &str) -> Result<String> {
        let index = self
            .descriptor
            .contents
            .iter()
            .position(|content| content == id)
            .ok_or_else(|| Error::Compatibility("The Kindle chapter does not exist.".into()))?;
        String::from_utf8(
            self.backend
                .load_raw(self.backend.spine()[index].id)
                .map_err(kindle_error)?,
        )
        .map_err(|error| {
            Error::Compatibility(format!("The decoded Kindle chapter is not UTF-8: {error}"))
        })
    }

    fn asset(&mut self, id: &str) -> Result<BookAsset> {
        Ok(BookAsset::new(id, self.container.asset(id)?))
    }
}
