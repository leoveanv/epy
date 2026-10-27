use super::number;
use crate::{Error, Result};
use std::io::Read;

// #NOTE: PalmDOC and FONT decoding adapt boko 0.5.0's format algorithms; see BOKO-NOTICE and BOKO-LICENSE.
fn invalid_compression() -> Error {
    Error::Compatibility("The compressed Kindle record is invalid or exceeds its limit.".into())
}

pub(super) fn palmdoc(bytes: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let byte = bytes[position];
        position += 1;
        match byte {
            0 | 9..=127 => output.push(byte),
            1..=8 => {
                let end = position + byte as usize;
                output.extend_from_slice(bytes.get(position..end).ok_or_else(invalid_compression)?);
                position = end;
            }
            128..=191 => {
                let next = *bytes.get(position).ok_or_else(invalid_compression)? as usize;
                position += 1;
                let distance = ((byte as usize & 63) << 5) | (next >> 3);
                if distance == 0 || distance > output.len() {
                    return Err(invalid_compression());
                }
                for _ in 0..(next & 7) + 3 {
                    output.push(output[output.len() - distance]);
                }
            }
            192..=255 => {
                output.push(b' ');
                output.push(byte ^ 128);
            }
        }
        if output.len() > limit {
            return Err(invalid_compression());
        }
    }
    Ok(output)
}

pub(super) fn strip_trailing_data(bytes: &[u8], flags: u16) -> Result<&[u8]> {
    let mut end = bytes.len();
    for bit in 1..16 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        let mut size = 0usize;
        let mut position = end;
        let mut encoded_bytes = 0;
        for shift in [0, 7, 14, 21] {
            position = position.checked_sub(1).ok_or_else(invalid_compression)?;
            let byte = bytes[position];
            size |= ((byte & 127) as usize) << shift;
            encoded_bytes += 1;
            if byte & 128 != 0 {
                break;
            }
            if shift == 21 {
                return Err(invalid_compression());
            }
        }
        if size < encoded_bytes || size > end {
            return Err(invalid_compression());
        }
        end -= size;
    }
    if flags & 1 != 0 {
        let last = *bytes
            .get(end.checked_sub(1).ok_or_else(invalid_compression)?)
            .ok_or_else(invalid_compression)?;
        end = end
            .checked_sub((last & 3) as usize + 1)
            .ok_or_else(invalid_compression)?;
    }
    Ok(&bytes[..end])
}

pub(super) fn font(bytes: &[u8], limit: u64) -> Result<Vec<u8>> {
    if bytes.get(..4) != Some(b"FONT") || bytes.len() < 24 {
        return Err(invalid_compression());
    }
    let flags = number(bytes, 8)?;
    let start = number(bytes, 12)? as usize;
    let mut data = bytes.get(start..).ok_or_else(invalid_compression)?.to_vec();
    if flags & 2 != 0 {
        let length = number(bytes, 16)? as usize;
        let start = number(bytes, 20)? as usize;
        let end = start
            .checked_add(length)
            .filter(|end| length > 0 && *end <= bytes.len())
            .ok_or_else(invalid_compression)?;
        let key = &bytes[start..end];
        for (index, byte) in data.iter_mut().take(1040).enumerate() {
            *byte ^= key[index % key.len()];
        }
    }
    if flags & 1 != 0 {
        let mut output = Vec::new();
        flate2::read::ZlibDecoder::new(data.as_slice())
            .take(limit.saturating_add(1))
            .read_to_end(&mut output)?;
        if output.len() as u64 > limit {
            return Err(invalid_compression());
        }
        Ok(output)
    } else if data.len() as u64 > limit {
        Err(invalid_compression())
    } else {
        Ok(data)
    }
}
