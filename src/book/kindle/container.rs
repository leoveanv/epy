use super::extended::extended_records;
use super::flows::read_flows;
use super::header::{Compression, MobiHeader, NULL_INDEX};
use super::huffman::load_dictionary;
use super::metadata::decode_metadata;
use super::number;
use super::source::RecordSource;
use crate::book::{BookMetadata, ResourceLimits};
use crate::{Error, Result};
use std::ops::Range;

pub(super) struct KindleContainer {
    pub records: RecordSource,
    pub header: MobiHeader,
    pub base: usize,
    pub metadata: BookMetadata,
    pub resource_ranges: Vec<Range<usize>>,
    pub flows: Vec<Range<usize>>,
    pub limits: ResourceLimits,
}

fn validate_header(bytes: &[u8], count: usize, limits: ResourceLimits) -> Result<MobiHeader> {
    if bytes.len() < 16 {
        return Err(Error::Compatibility(
            "The Kindle text header is truncated.".into(),
        ));
    }
    if bytes[12..14] != [0, 0] {
        return Err(Error::Compatibility(
            "DRM-protected Kindle books are not supported.".into(),
        ));
    }
    if number(bytes, 4)? as u64 > limits.archive_bytes_max
        || u16::from_be_bytes([bytes[8], bytes[9]]) as usize >= count
    {
        return Err(Error::Compatibility(
            "The Kindle text exceeds its declared bounds.".into(),
        ));
    }
    if bytes.len() > 16
        && (bytes.get(16..20) != Some(b"MOBI")
            || bytes.len() < 32
            || (number(bytes, 20)? as usize)
                .checked_add(16)
                .is_none_or(|end| end > bytes.len()))
    {
        return Err(Error::Compatibility(
            "The Kindle MOBI header length exceeds its record.".into(),
        ));
    }
    if bytes.len() >= 32 && !matches!(number(bytes, 28)?, 1252 | 65001) {
        return Err(Error::Compatibility(
            "The Kindle text encoding is unsupported.".into(),
        ));
    }
    let header = MobiHeader::parse(bytes)?;
    if !matches!(
        header.compression,
        Compression::None | Compression::PalmDoc | Compression::Huffman
    ) {
        return Err(Error::Compatibility(
            "The Kindle text compression is unsupported.".into(),
        ));
    }
    if header.compression == Compression::Huffman
        && (header.huff_record_index <= header.text_record_count as u32
            || header.huff_record_count < 2
            || (header.huff_record_index as usize)
                .checked_add(header.huff_record_count as usize)
                .is_none_or(|end| end > count))
    {
        return Err(Error::Compatibility(
            "The Kindle Huffman dictionaries are missing or invalid.".into(),
        ));
    }
    Ok(header)
}

fn combined_base(bytes: &[u8], records: &RecordSource) -> Result<Option<usize>> {
    let mut base = None;
    for record in extended_records(bytes)? {
        if record.kind != 121 {
            continue;
        }
        let index = number(record.bytes, 0)?;
        if index == NULL_INDEX {
            continue;
        }
        let index = index as usize;
        if base.is_some_and(|base| base != index)
            || index < 2
            || index >= records.count()
            || !records.read(index - 1)?.starts_with(b"BOUNDARY")
        {
            return Err(Error::Compatibility(
                "The combined Kindle boundary is missing or invalid.".into(),
            ));
        }
        base = Some(index);
    }
    Ok(base)
}

fn resource_range(header: &MobiHeader, base: usize, end: usize) -> Result<Range<usize>> {
    if header.first_image_index == NULL_INDEX {
        return Ok(end..end);
    }
    let start = base
        .checked_add(header.first_image_index as usize)
        .filter(|start| *start <= end)
        .ok_or_else(|| {
            Error::Compatibility("The Kindle resource index exceeds its record range.".into())
        })?;
    Ok(start..end)
}

impl KindleContainer {
    pub fn open(records: RecordSource, limits: ResourceLimits) -> Result<Self> {
        let first = records.read(0)?;
        let base = combined_base(&first, &records)?.unwrap_or(0);
        let original = validate_header(
            &first,
            if base > 0 { base - 1 } else { records.count() },
            limits,
        )?;
        let selected = if base == 0 {
            first
        } else {
            records.read(base)?
        };
        let header = validate_header(&selected, records.count() - base, limits)?;
        if base > 0 && header.mobi_version != 8 {
            return Err(Error::Compatibility(
                "The combined Kindle rendition is not KF8.".into(),
            ));
        }
        if header.compression == Compression::Huffman {
            load_dictionary(&records, &header, base)?;
        }
        let metadata = decode_metadata(&selected, &records.name)?;
        let flows = read_flows(&records, &header, base, limits)?;
        let mut resource_ranges = Vec::new();
        if base > 0 {
            // #NOTE: Combined books number shared resources before the KF8 rendition's own resources.
            resource_ranges.push(resource_range(&original, 0, base - 1)?);
        }
        resource_ranges.push(resource_range(&header, base, records.count())?);
        Ok(Self {
            records,
            header,
            base,
            metadata,
            resource_ranges,
            flows,
            limits,
        })
    }
}
