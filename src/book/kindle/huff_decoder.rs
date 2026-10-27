use super::number;
use crate::{Error, Result};

// #NOTE: The HUFF/CDIC format algorithm follows boko 0.5.0; see BOKO-NOTICE and BOKO-LICENSE.
#[derive(Clone, Copy)]
struct PrefixCode {
    bits: usize,
    terminal: bool,
    maximum: u32,
}

struct Phrase {
    bytes: Vec<u8>,
    leaf: bool,
    decoded: Option<Vec<u8>>,
}

pub(super) struct HuffCdicReader {
    prefixes: Vec<PrefixCode>,
    minimum: Vec<u32>,
    maximum: Vec<u32>,
    phrases: Vec<Phrase>,
}

fn invalid_huffman() -> Error {
    Error::Compatibility("The Kindle Huffman stream is invalid or exceeds its limit.".into())
}

fn maximum_code(raw: u32, bits: usize) -> u32 {
    raw.wrapping_add(1)
        .wrapping_shl((32 - bits) as u32)
        .wrapping_sub(1)
}

fn prefixes(huff: &[u8]) -> Result<Vec<PrefixCode>> {
    let start = number(huff, 8)? as usize;
    let end = start
        .checked_add(1024)
        .filter(|end| *end <= huff.len())
        .ok_or_else(invalid_huffman)?;
    huff[start..end]
        .chunks_exact(4)
        .map(|bytes| {
            let entry = number(bytes, 0)?;
            let bits = (entry & 31) as usize;
            if bits == 0 {
                return Err(invalid_huffman());
            }
            Ok(PrefixCode {
                bits,
                terminal: entry & 0x80 != 0,
                maximum: maximum_code(entry >> 8, bits),
            })
        })
        .collect()
}

fn code_at(bytes: &[u8], position_bits: usize) -> u32 {
    let position = position_bits / 8;
    let mut window = 0u64;
    for index in position..position + 5 {
        window = (window << 8) | bytes.get(index).copied().unwrap_or_default() as u64;
    }
    (window >> (8 - position_bits % 8)) as u32
}

fn append(output: &mut Vec<u8>, bytes: &[u8], budget: &mut usize) -> Result<()> {
    *budget = budget
        .checked_sub(bytes.len())
        .ok_or_else(invalid_huffman)?;
    output.extend_from_slice(bytes);
    Ok(())
}

impl HuffCdicReader {
    pub fn new(huff: &[u8], dictionaries: &[&[u8]]) -> Result<Self> {
        if huff.get(..8) != Some(b"HUFF\0\0\0\x18") {
            return Err(invalid_huffman());
        }
        let prefixes = prefixes(huff)?;
        let start = number(huff, 12)? as usize;
        let end = start
            .checked_add(256)
            .filter(|end| *end <= huff.len())
            .ok_or_else(invalid_huffman)?;
        let mut minimum = vec![0];
        let mut maximum = vec![0];
        for (index, bytes) in huff[start..end].chunks_exact(8).enumerate() {
            let bits = index + 1;
            minimum.push(number(bytes, 0)?.wrapping_shl((32 - bits) as u32));
            maximum.push(maximum_code(number(bytes, 4)?, bits));
        }
        let mut phrases = Vec::new();
        for dictionary in dictionaries {
            let count = (1usize << number(dictionary, 12)?)
                .min(number(dictionary, 8)? as usize - phrases.len());
            for index in 0..count {
                let position = 16
                    + u16::from_be_bytes([dictionary[16 + index * 2], dictionary[17 + index * 2]])
                        as usize;
                let flags = u16::from_be_bytes([dictionary[position], dictionary[position + 1]]);
                let length = (flags & 0x7fff) as usize;
                phrases.push(Phrase {
                    bytes: dictionary[position + 2..position + 2 + length].to_vec(),
                    leaf: flags & 0x8000 != 0,
                    decoded: None,
                });
            }
        }
        Ok(Self {
            prefixes,
            minimum,
            maximum,
            phrases,
        })
    }

    pub fn decompress(&mut self, bytes: &[u8], budget: &mut usize) -> Result<Vec<u8>> {
        let allowed = (*budget).min(16 * 1024 * 1024);
        let mut remaining = allowed;
        let mut output = Vec::new();
        self.unpack(bytes, &mut output, &mut remaining, 0)?;
        *budget -= allowed - remaining;
        Ok(output)
    }

    fn phrase(
        &mut self,
        index: usize,
        output: &mut Vec<u8>,
        budget: &mut usize,
        depth: usize,
    ) -> Result<()> {
        let phrase = self.phrases.get(index).ok_or_else(invalid_huffman)?;
        if phrase.leaf {
            return append(output, &phrase.bytes, budget);
        }
        if let Some(decoded) = &phrase.decoded {
            return append(output, decoded, budget);
        }
        let encoded = phrase.bytes.clone();
        let mut decoded = Vec::new();
        self.unpack(&encoded, &mut decoded, budget, depth + 1)?;
        output.extend_from_slice(&decoded);
        self.phrases[index].decoded = Some(decoded);
        Ok(())
    }

    fn unpack(
        &mut self,
        bytes: &[u8],
        output: &mut Vec<u8>,
        budget: &mut usize,
        depth: usize,
    ) -> Result<()> {
        if depth > 32 {
            return Err(invalid_huffman());
        }
        let mut position = 0;
        while position < bytes.len() * 8 {
            let code = code_at(bytes, position);
            let prefix = self.prefixes[(code >> 24) as usize];
            let mut bits = prefix.bits;
            let mut maximum = prefix.maximum;
            if !prefix.terminal {
                while bits < 32 && code < self.minimum[bits] {
                    bits += 1;
                }
                maximum = self.maximum[bits];
            }
            position += bits;
            if position > bytes.len() * 8 {
                break;
            }
            let index = (maximum.wrapping_sub(code) >> (32 - bits)) as usize;
            self.phrase(index, output, budget, depth)?;
        }
        Ok(())
    }
}
