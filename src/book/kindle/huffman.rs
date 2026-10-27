use super::header::MobiHeader;
use super::huff_decoder::HuffCdicReader;
use super::number;
use super::source::RecordSource;
use crate::{Error, Result};

fn invalid_dictionary() -> Error {
    Error::Compatibility("The Kindle Huffman dictionary is incomplete or invalid.".into())
}

fn validate_phrases(dictionary: &[u8], loaded: usize, expected: Option<usize>) -> Result<usize> {
    if dictionary.get(..8) != Some(b"CDIC\0\0\0\x10") {
        return Err(invalid_dictionary());
    }
    let phrases = number(dictionary, 8)? as usize;
    let bits = number(dictionary, 12)?;
    if phrases == 0
        || expected.is_some_and(|expected| expected != phrases)
        || bits > 16
        || loaded >= phrases
    {
        return Err(invalid_dictionary());
    }
    let count = (1usize << bits).min(phrases - loaded);
    let table_end = 16 + count * 2;
    if dictionary.len() < table_end {
        return Err(invalid_dictionary());
    }
    for offset in dictionary[16..table_end].chunks_exact(2) {
        let position = 16 + u16::from_be_bytes([offset[0], offset[1]]) as usize;
        let length_bytes = dictionary
            .get(position..)
            .and_then(|tail| tail.get(..2))
            .ok_or_else(invalid_dictionary)?;
        let length = (u16::from_be_bytes([length_bytes[0], length_bytes[1]]) & 0x7fff) as usize;
        if position < table_end || position + 2 + length > dictionary.len() {
            return Err(invalid_dictionary());
        }
    }
    Ok(count)
}

pub(super) fn load_dictionary(
    records: &RecordSource,
    header: &MobiHeader,
    base: usize,
) -> Result<HuffCdicReader> {
    let start = base
        .checked_add(header.huff_record_index as usize)
        .ok_or_else(invalid_dictionary)?;
    let count = header.huff_record_count as usize;
    if count < 2
        || start
            .checked_add(count)
            .is_none_or(|end| end > records.count())
    {
        return Err(invalid_dictionary());
    }
    let huff = records.read(start)?;
    let mut dictionaries = Vec::with_capacity(count - 1);
    let mut loaded = 0;
    let mut expected = None;
    for index in start + 1..start + count {
        let dictionary = records.read(index)?;
        loaded += validate_phrases(&dictionary, loaded, expected)?;
        expected = Some(number(&dictionary, 8)? as usize);
        dictionaries.push(dictionary);
    }
    if expected != Some(loaded) {
        return Err(invalid_dictionary());
    }
    HuffCdicReader::new(
        &huff,
        &dictionaries.iter().map(Vec::as_slice).collect::<Vec<_>>(),
    )
}
