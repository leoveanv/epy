use super::compression::{palmdoc, strip_trailing_data};
use super::container::KindleContainer;
use super::header::Compression;
use super::huff_decoder::HuffCdicReader;
use super::huffman::load_dictionary;
use super::source::LimitedSource;
use crate::cancellation::Cancellation;
use crate::{Error, Result};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::sync::Arc;

struct TextDecoder<'a> {
    container: &'a KindleContainer,
    dictionary: Option<HuffCdicReader>,
    remaining: usize,
}

impl<'a> TextDecoder<'a> {
    fn new(container: &'a KindleContainer) -> Result<Self> {
        Ok(Self {
            container,
            dictionary: if container.header.compression == Compression::Huffman {
                Some(load_dictionary(
                    &container.records,
                    &container.header,
                    container.base,
                )?)
            } else {
                None
            },
            remaining: usize::try_from(container.limits.archive_bytes_max).unwrap_or(usize::MAX),
        })
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        let bytes = strip_trailing_data(bytes, self.container.header.extra_data_flags)?;
        let limit = usize::try_from(self.container.limits.resource_bytes_max).unwrap_or(usize::MAX);
        let decoded = match self.container.header.compression {
            Compression::None => bytes.to_vec(),
            Compression::PalmDoc => palmdoc(bytes, limit)?,
            Compression::Huffman => self
                .dictionary
                .as_mut()
                .ok_or_else(|| Error::Compatibility("The Kindle dictionary is missing.".into()))?
                .decompress(bytes, &mut self.remaining)?,
            Compression::Unknown(_) => {
                return Err(Error::Compatibility(
                    "The Kindle compression is unsupported.".into(),
                ));
            }
        };
        if decoded.len() > limit {
            return Err(Error::Compatibility(
                "The decoded Kindle record exceeds its limit.".into(),
            ));
        }
        if self.container.header.compression != Compression::Huffman {
            self.remaining = self.remaining.checked_sub(decoded.len()).ok_or_else(|| {
                Error::Compatibility("The decoded Kindle text exceeds its limit.".into())
            })?;
        }
        Ok(decoded)
    }
}

fn rename_aid(tag: &mut [u8]) {
    if tag.get(1).is_none_or(|byte| !byte.is_ascii_alphabetic()) {
        return;
    }
    let mut position = 1;
    while position < tag.len() && !tag[position].is_ascii_whitespace() && tag[position] != b'>' {
        position += 1;
    }
    let mut aid = None;
    let mut has_id = false;
    while position < tag.len() {
        while position < tag.len() && tag[position].is_ascii_whitespace() {
            position += 1;
        }
        if position == tag.len() || matches!(tag[position], b'>' | b'/') {
            break;
        }
        let start = position;
        while position < tag.len()
            && !tag[position].is_ascii_whitespace()
            && !matches!(tag[position], b'=' | b'>' | b'/')
        {
            position += 1;
        }
        if tag[start..position].eq_ignore_ascii_case(b"aid") {
            aid = Some(start);
        }
        if tag[start..position].eq_ignore_ascii_case(b"id") {
            has_id = true;
        }
        while position < tag.len() && tag[position].is_ascii_whitespace() {
            position += 1;
        }
        if tag.get(position) != Some(&b'=') {
            continue;
        }
        position += 1;
        while position < tag.len() && tag[position].is_ascii_whitespace() {
            position += 1;
        }
        if let Some(quote @ (b'\'' | b'"')) = tag.get(position).copied() {
            position += 1;
            while position < tag.len() && tag[position] != quote {
                position += 1;
            }
            position += usize::from(position < tag.len());
        } else {
            while position < tag.len()
                && !tag[position].is_ascii_whitespace()
                && tag[position] != b'>'
            {
                position += 1;
            }
        }
    }
    if let Some(position) = aid {
        // #NOTE: Equal-length names preserve every KF8 skeleton, fragment, and NCX byte position.
        tag[position..position + 3].copy_from_slice(if has_id { b"_ai" } else { b" id" });
    }
}

struct TagRewriter {
    pending: Vec<u8>,
    quote: Option<u8>,
    limit: u64,
}

impl TagRewriter {
    fn new(limit: u64) -> Self {
        Self {
            pending: Vec::new(),
            quote: None,
            limit,
        }
    }

    fn write(&mut self, bytes: &[u8], output: &mut impl Write) -> Result<()> {
        let mut position = 0;
        while position < bytes.len() {
            if self.pending.is_empty() {
                let next = bytes[position..]
                    .iter()
                    .position(|byte| *byte == b'<')
                    .map_or(bytes.len(), |next| position + next);
                output.write_all(&bytes[position..next])?;
                position = next;
                if position == bytes.len() {
                    break;
                }
            }
            let byte = bytes[position];
            self.pending.push(byte);
            position += 1;
            if self.pending.len() as u64 > self.limit {
                return Err(Error::Compatibility(
                    "A Kindle markup tag exceeds its size limit.".into(),
                ));
            }
            if self.quote == Some(byte) {
                self.quote = None;
            } else if self.quote.is_none() && matches!(byte, b'\'' | b'"') {
                self.quote = Some(byte);
            } else if self.quote.is_none() && byte == b'>' {
                rename_aid(&mut self.pending);
                output.write_all(&self.pending)?;
                self.pending.clear();
            }
        }
        Ok(())
    }

    fn finish(&mut self, output: &mut impl Write) -> Result<()> {
        output.write_all(&self.pending)?;
        self.pending.clear();
        Ok(())
    }
}

pub(super) fn prepare_kf8(
    container: &KindleContainer,
    cancellation: Cancellation,
) -> Result<Arc<LimitedSource>> {
    let mut prefix = container.records.prefix()?;
    let mut position = prefix.len() as u64;
    let mut output = BufWriter::new(tempfile::tempfile()?);
    output.write_all(&prefix)?;
    let mut decoder = TextDecoder::new(container)?;
    let mut rewriter = TagRewriter::new(container.limits.resource_bytes_max);
    let text_start = container.base + 1;
    let text_end = text_start + container.header.text_record_count as usize;
    for index in 0..container.records.count() {
        cancellation.check()?;
        let offset = u32::try_from(position).map_err(|_| {
            Error::Compatibility("The prepared Kindle record offset overflows.".into())
        })?;
        prefix[78 + index * 8..82 + index * 8].copy_from_slice(&offset.to_be_bytes());
        let length = if index == container.base {
            let mut bytes = container.records.read(index)?;
            bytes[..2].copy_from_slice(&1u16.to_be_bytes());
            if bytes.len() >= 0xf4 {
                bytes[0xf2..0xf4].copy_from_slice(&0u16.to_be_bytes());
            }
            output.write_all(&bytes)?;
            bytes.len() as u64
        } else if (text_start..text_end).contains(&index) {
            let bytes = decoder.decode(&container.records.read(index)?)?;
            rewriter.write(&bytes, &mut output)?;
            if index + 1 == text_end {
                rewriter.finish(&mut output)?;
            }
            bytes.len() as u64
        } else {
            container.records.copy(index, &mut output)?
        };
        position = position
            .checked_add(length)
            .filter(|position| *position <= container.limits.archive_bytes_max)
            .ok_or_else(|| {
                Error::Compatibility("The prepared Kindle file exceeds its size limit.".into())
            })?;
    }
    output.seek(SeekFrom::Start(0))?;
    output.write_all(&prefix)?;
    let file = output.into_inner().map_err(|error| error.into_error())?;
    LimitedSource::from_file(file, container.limits, cancellation)
}
