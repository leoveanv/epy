use super::number;
use crate::Result;

pub(super) const NULL_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Compression {
    None,
    PalmDoc,
    Huffman,
    Unknown(u16),
}

pub(super) struct MobiHeader {
    pub compression: Compression,
    pub text_record_count: u16,
    pub text_length: u32,
    pub mobi_version: u32,
    pub first_image_index: u32,
    pub huff_record_index: u32,
    pub huff_record_count: u32,
    pub fdst_index: u32,
    pub extra_data_flags: u16,
}

fn optional_number(bytes: &[u8], offset: usize, default: u32) -> Result<u32> {
    if bytes.len() < offset + 4 {
        Ok(default)
    } else {
        number(bytes, offset)
    }
}

impl MobiHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let compression = match u16::from_be_bytes([bytes[0], bytes[1]]) {
            1 => Compression::None,
            2 => Compression::PalmDoc,
            0x4448 => Compression::Huffman,
            value => Compression::Unknown(value),
        };
        let header_length = optional_number(bytes, 20, 0)?;
        Ok(Self {
            compression,
            text_record_count: u16::from_be_bytes([bytes[8], bytes[9]]),
            text_length: number(bytes, 4)?,
            mobi_version: optional_number(bytes, 0x68, 1)?,
            first_image_index: optional_number(bytes, 0x6c, NULL_INDEX)?,
            huff_record_index: optional_number(bytes, 0x70, NULL_INDEX)?,
            huff_record_count: optional_number(bytes, 0x74, 0)?,
            fdst_index: optional_number(bytes, 0xc0, NULL_INDEX)?,
            extra_data_flags: if bytes.len() >= 0xf4 && header_length >= 0xe4 {
                u16::from_be_bytes([bytes[0xf2], bytes[0xf3]])
            } else {
                0
            },
        })
    }
}
