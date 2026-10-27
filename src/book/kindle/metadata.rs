use super::extended::extended_records;
use super::number;
use crate::book::BookMetadata;
use crate::{Error, Result};

fn decode(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> Result<String> {
    let (value, errors) = encoding.decode_without_bom_handling(bytes);
    if errors {
        return Err(Error::Compatibility(
            "The Kindle metadata contains invalid encoded text.".into(),
        ));
    }
    Ok(value.trim_end_matches('\0').to_owned())
}

pub(super) fn decode_metadata(header: &[u8], pdb_name: &[u8]) -> Result<BookMetadata> {
    let encoding = if header.len() >= 32 && number(header, 28)? == 65001 {
        encoding_rs::UTF_8
    } else {
        encoding_rs::WINDOWS_1252
    };
    let mut metadata = BookMetadata::default();
    if header.len() >= 0x5c {
        let start = number(header, 0x54)? as usize;
        let length = number(header, 0x58)? as usize;
        if length > 0 {
            let end = start
                .checked_add(length)
                .ok_or_else(|| Error::Compatibility("The Kindle title length overflows.".into()))?;
            metadata.title = Some(decode(
                header.get(start..end).ok_or_else(|| {
                    Error::Compatibility("The Kindle title is outside its header.".into())
                })?,
                encoding,
            )?);
        }
    }
    let mut authors = Vec::new();
    for record in extended_records(header)? {
        if !matches!(
            record.kind,
            100 | 101 | 103 | 104 | 106 | 112 | 113 | 503 | 524
        ) {
            continue;
        }
        let value = decode(record.bytes, encoding)?;
        if !value.is_empty() {
            match record.kind {
                100 => authors.push(value),
                101 => metadata.publisher = Some(value),
                103 => metadata.description = Some(value),
                104 | 113 => metadata.identifier = Some(value),
                106 => metadata.date = Some(value),
                112 => metadata.source = Some(value),
                503 => metadata.title = Some(value),
                524 => metadata.language = Some(value),
                _ => {}
            }
        }
    }
    if !authors.is_empty() {
        metadata.creator = Some(authors.join(", "));
    }
    if metadata.title.as_ref().is_none_or(|title| title.is_empty()) {
        let name = decode(
            pdb_name.split(|byte| *byte == 0).next().unwrap_or_default(),
            encoding,
        )?;
        metadata.title = (!name.is_empty()).then_some(name);
    }
    Ok(metadata)
}
