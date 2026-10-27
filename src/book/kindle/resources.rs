use super::compression::{font, palmdoc, strip_trailing_data};
use super::container::KindleContainer;
use super::header::Compression;
use super::huffman::load_dictionary;
use crate::{Error, Result};

fn resource_index(path: &str, prefix: &str, extension: &str) -> Option<usize> {
    let digits = path.strip_prefix(prefix)?.strip_suffix(extension)?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

impl KindleContainer {
    pub fn asset(&self, path: &str) -> Result<Vec<u8>> {
        if let Some(index) = resource_index(path, "styles/style", ".css") {
            return self.flow(
                index.checked_add(1).ok_or_else(|| {
                    Error::Compatibility("The Kindle flow index overflows.".into())
                })?,
            );
        }
        let stem = path
            .strip_prefix("images/image_")
            .or_else(|| path.strip_prefix("fonts/font_"));
        let index = stem
            .and_then(|stem| stem.rsplit_once('.'))
            .and_then(|(digits, extension)| {
                (!extension.is_empty()
                    && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    && !digits.is_empty()
                    && digits.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| digits.parse::<usize>().ok())
                .flatten()
            })
            .ok_or_else(|| {
                Error::Compatibility("The Kindle resource identifier is invalid.".into())
            })?;
        let record = self
            .resource_ranges
            .iter()
            .flat_map(Clone::clone)
            .nth(index)
            .ok_or_else(|| Error::Compatibility("The Kindle resource does not exist.".into()))?;
        let bytes = self.records.read(record)?;
        let bytes = if bytes.starts_with(b"FONT") {
            font(&bytes, self.limits.resource_bytes_max)?
        } else {
            bytes
        };
        if bytes.len() as u64 > self.limits.resource_bytes_max {
            return Err(Error::Compatibility(
                "The decoded Kindle resource exceeds its limit.".into(),
            ));
        }
        Ok(bytes)
    }

    fn flow(&self, index: usize) -> Result<Vec<u8>> {
        let range = self
            .flows
            .get(index)
            .ok_or_else(|| Error::Compatibility("The Kindle flow does not exist.".into()))?;
        if range.len() as u64 > self.limits.resource_bytes_max {
            return Err(Error::Compatibility(
                "The Kindle flow exceeds its resource limit.".into(),
            ));
        }
        let mut dictionary = if self.header.compression == Compression::Huffman {
            Some(load_dictionary(&self.records, &self.header, self.base)?)
        } else {
            None
        };
        let mut budget = usize::try_from(self.limits.archive_bytes_max).unwrap_or(usize::MAX);
        let mut position = 0usize;
        let mut output = Vec::with_capacity(range.len());
        for index in 1..=self.header.text_record_count as usize {
            let bytes = self.records.read(self.base + index)?;
            let bytes = strip_trailing_data(&bytes, self.header.extra_data_flags)?;
            let decoded = match self.header.compression {
                Compression::None => bytes.to_vec(),
                Compression::PalmDoc => palmdoc(
                    bytes,
                    usize::try_from(self.limits.resource_bytes_max).unwrap_or(usize::MAX),
                )?,
                Compression::Huffman => dictionary
                    .as_mut()
                    .ok_or_else(|| {
                        Error::Compatibility("The Kindle dictionary is missing.".into())
                    })?
                    .decompress(bytes, &mut budget)?,
                Compression::Unknown(_) => {
                    return Err(Error::Compatibility(
                        "The Kindle compression is unsupported.".into(),
                    ));
                }
            };
            let end = position
                .checked_add(decoded.len())
                .filter(|end| *end as u64 <= self.limits.archive_bytes_max)
                .ok_or_else(|| {
                    Error::Compatibility("The decoded Kindle text exceeds its limit.".into())
                })?;
            let selected_start = range.start.saturating_sub(position).min(decoded.len());
            let selected_end = range.end.saturating_sub(position).min(decoded.len());
            output.extend_from_slice(&decoded[selected_start..selected_end]);
            position = end;
            if position >= range.end {
                return Ok(output);
            }
        }
        Err(Error::Compatibility(
            "The Kindle flow extends past its decoded text.".into(),
        ))
    }
}
