use super::number;
use crate::{Error, Result};

pub(super) struct ExtendedRecord<'a> {
    pub kind: u32,
    pub bytes: &'a [u8],
}

pub(super) fn extended_records(header: &[u8]) -> Result<Vec<ExtendedRecord<'_>>> {
    if header.len() < 0x84 || number(header, 0x80)? & 0x40 == 0 {
        return Ok(Vec::new());
    }
    let start = 16usize
        .checked_add(number(header, 20)? as usize)
        .ok_or_else(|| Error::Compatibility("The Kindle metadata offset overflows.".into()))?;
    if header.get(start..).and_then(|tail| tail.get(..4)) != Some(b"EXTH") {
        return Err(Error::Compatibility(
            "The declared Kindle metadata header is missing.".into(),
        ));
    }
    let length = number(header, start + 4)? as usize;
    let end = start
        .checked_add(length)
        .filter(|end| length >= 12 && *end <= header.len())
        .ok_or_else(|| {
            Error::Compatibility("The Kindle metadata region exceeds its record.".into())
        })?;
    let count = number(header, start + 8)? as usize;
    if count > 100_000 || count > (length - 12) / 8 {
        return Err(Error::Compatibility(
            "The Kindle metadata count exceeds its region.".into(),
        ));
    }
    let mut records = Vec::with_capacity(count);
    let mut position = start + 12;
    for _ in 0..count {
        let kind = number(&header[..end], position)?;
        let length = number(&header[..end], position + 4)? as usize;
        let next = position
            .checked_add(length)
            .filter(|next| length >= 8 && *next <= end)
            .ok_or_else(|| {
                Error::Compatibility(
                    "The Kindle metadata record exceeds its declared region.".into(),
                )
            })?;
        records.push(ExtendedRecord {
            kind,
            bytes: &header[position + 8..next],
        });
        position = next;
    }
    Ok(records)
}
