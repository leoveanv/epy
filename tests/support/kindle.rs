use std::path::{Path, PathBuf};

pub struct PalmDatabase {
    pub records: Vec<Vec<u8>>,
}

impl PalmDatabase {
    pub fn new(records: Vec<Vec<u8>>) -> Self {
        Self { records }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let count = u16::from_be_bytes(bytes[76..78].try_into().unwrap()) as usize;
        let offsets = (0..count)
            .map(|index| {
                u32::from_be_bytes(bytes[78 + index * 8..82 + index * 8].try_into().unwrap())
                    as usize
            })
            .chain(std::iter::once(bytes.len()))
            .collect::<Vec<_>>();
        Self::new(
            offsets
                .windows(2)
                .map(|range| bytes[range[0]..range[1]].to_vec())
                .collect(),
        )
    }

    pub fn write(&self, directory: &Path, extension: &str) -> PathBuf {
        let first = 78 + self.records.len() * 8 + 2;
        let mut bytes = vec![0u8; first];
        bytes[..16].copy_from_slice(b"Synthetic Kindle");
        bytes[60..68].copy_from_slice(b"BOOKMOBI");
        bytes[76..78].copy_from_slice(&(self.records.len() as u16).to_be_bytes());
        for (index, record) in self.records.iter().enumerate() {
            let offset = bytes.len() as u32;
            bytes[78 + index * 8..82 + index * 8].copy_from_slice(&offset.to_be_bytes());
            bytes.extend(record);
        }
        let path = directory.join(format!("synthetic.{extension}"));
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

pub struct ExtendedRecord {
    kind: u32,
    data: Vec<u8>,
}

impl ExtendedRecord {
    pub fn new(kind: u32, data: impl Into<Vec<u8>>) -> Self {
        Self {
            kind,
            data: data.into(),
        }
    }
}

pub struct MobiBuilder {
    pub compression: u16,
    pub encoding: u32,
    pub header_length: u32,
    pub encrypted: bool,
    pub title: String,
    pub html: Vec<u8>,
    pub extended: Vec<ExtendedRecord>,
}

impl MobiBuilder {
    pub fn new() -> Self {
        Self { compression: 1, encoding: 65001, header_length: 232, encrypted: false, title: "Synthetic Kindle".into(), html: "<html><head><title>Sample</title></head><body><h1>First chapter</h1><p>Café 👩‍🔬 first passage.</p><mbp:pagebreak/><h1>Second chapter</h1><p>Last passage.</p></body></html>".as_bytes().to_vec(), extended: Vec::new() }
    }

    pub fn database(&self) -> PalmDatabase {
        let title = if self.encoding == 1252 {
            encoding_rs::WINDOWS_1252.encode(&self.title).0.into_owned()
        } else {
            self.title.as_bytes().to_vec()
        };
        let mut extended = Vec::new();
        if !self.extended.is_empty() {
            extended.extend(b"EXTH\0\0\0\0");
            extended.extend((self.extended.len() as u32).to_be_bytes());
            for record in &self.extended {
                extended.extend(record.kind.to_be_bytes());
                extended.extend((8 + record.data.len() as u32).to_be_bytes());
                extended.extend(&record.data);
            }
            let length = extended.len() as u32;
            extended[4..8].copy_from_slice(&length.to_be_bytes());
        }
        let title_offset = (248 + extended.len()).max(256).next_multiple_of(4);
        let mut header = vec![0u8; title_offset + title.len()];
        header[..2].copy_from_slice(&self.compression.to_be_bytes());
        header[4..8].copy_from_slice(&(self.html.len() as u32).to_be_bytes());
        header[8..10].copy_from_slice(&1u16.to_be_bytes());
        header[10..12].copy_from_slice(&4096u16.to_be_bytes());
        header[12..14].copy_from_slice(&u16::from(self.encrypted).to_be_bytes());
        header[16..20].copy_from_slice(b"MOBI");
        header[20..24].copy_from_slice(&self.header_length.to_be_bytes());
        header[24..28].copy_from_slice(&2u32.to_be_bytes());
        header[28..32].copy_from_slice(&self.encoding.to_be_bytes());
        header[0x54..0x58].copy_from_slice(&(title_offset as u32).to_be_bytes());
        header[0x58..0x5c].copy_from_slice(&(title.len() as u32).to_be_bytes());
        header[0x68..0x6c].copy_from_slice(&6u32.to_be_bytes());
        for offset in [0x6c, 0x70, 0xf4] {
            header[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        if !extended.is_empty() {
            header[0x80..0x84].copy_from_slice(&0x40u32.to_be_bytes());
            header[248..248 + extended.len()].copy_from_slice(&extended);
        }
        header[title_offset..].copy_from_slice(&title);
        let text = if self.compression == 2 {
            self.html
                .chunks(8)
                .flat_map(|chunk| std::iter::once(chunk.len() as u8).chain(chunk.iter().copied()))
                .collect::<Vec<_>>()
        } else {
            self.html.clone()
        };
        PalmDatabase::new(vec![header, text])
    }

    pub fn build(&self, root: &Path, extension: &str) -> PathBuf {
        self.database().write(root, extension)
    }
}

struct DictionaryPhrase {
    bytes: Vec<u8>,
    leaf: bool,
}

impl DictionaryPhrase {
    fn new(bytes: &[u8], leaf: bool) -> Self {
        Self {
            bytes: bytes.to_vec(),
            leaf,
        }
    }
}

pub struct HuffmanBuilder {
    pub recursive: bool,
    pub truncate_phrase: bool,
    pub dictionary_bits: u32,
    pub zero_code_length: bool,
    pub trailing_header_bytes: usize,
}

impl HuffmanBuilder {
    pub fn new() -> Self {
        Self {
            recursive: false,
            truncate_phrase: false,
            dictionary_bits: 1,
            zero_code_length: false,
            trailing_header_bytes: 0,
        }
    }

    fn huff_record(&self) -> Vec<u8> {
        let mut huff = vec![0u8; 24];
        huff[..8].copy_from_slice(b"HUFF\0\0\0\x18");
        huff[8..12].copy_from_slice(&24u32.to_be_bytes());
        huff[12..16].copy_from_slice(&1048u32.to_be_bytes());
        for _ in 0..256 {
            let value = if self.zero_code_length {
                0xff80u32
            } else {
                0xff88
            };
            huff.extend(value.to_be_bytes());
        }
        for length in 1..=32 {
            huff.extend(0u32.to_be_bytes());
            huff.extend(if length == 8 { 255u32 } else { 0 }.to_be_bytes());
        }
        huff
    }

    pub fn encode_modern(&self, mut database: PalmDatabase) -> PalmDatabase {
        let count = u16::from_be_bytes(database.records[0][8..10].try_into().unwrap()) as usize;
        let phrases = database.records[1..=count]
            .iter()
            .map(|record| {
                DictionaryPhrase::new(
                    &if database.records[0][..2] == [0, 2] {
                        decode_fixture_palmdoc(strip_fixture_trailer(
                            record,
                            u16::from_be_bytes(database.records[0][0xf2..0xf4].try_into().unwrap()),
                        ))
                    } else {
                        record.clone()
                    },
                    true,
                )
            })
            .collect::<Vec<_>>();
        assert!(count < 256);
        for index in 0..count {
            database.records[index + 1] = vec![255 - index as u8, 0];
        }
        let dictionary_start = database.records.len() as u32;
        database.records.push(self.huff_record());
        for chunk in phrases.chunks(2) {
            let mut dictionary = vec![0u8; 16 + chunk.len() * 2];
            dictionary[..8].copy_from_slice(b"CDIC\0\0\0\x10");
            dictionary[8..12].copy_from_slice(&(count as u32).to_be_bytes());
            dictionary[12..16].copy_from_slice(&1u32.to_be_bytes());
            for (index, phrase) in chunk.iter().enumerate() {
                let offset = (dictionary.len() - 16) as u16;
                dictionary[16 + index * 2..18 + index * 2].copy_from_slice(&offset.to_be_bytes());
                dictionary.extend((0x8000 | phrase.bytes.len() as u16).to_be_bytes());
                dictionary.extend(&phrase.bytes);
            }
            database.records.push(dictionary);
        }
        let dictionary_count = database.records.len() as u32 - dictionary_start;
        database.records[0][..2].copy_from_slice(&0x4448u16.to_be_bytes());
        database.records[0][0xf2..0xf4].copy_from_slice(&1u16.to_be_bytes());
        database.records[0][0x70..0x74].copy_from_slice(&dictionary_start.to_be_bytes());
        database.records[0][0x74..0x78].copy_from_slice(&dictionary_count.to_be_bytes());
        database
    }

    pub fn database(&self) -> PalmDatabase {
        let mut mobi = MobiBuilder::new();
        mobi.html = "<html><body><p>Café 👩‍🔬 Huffman passage.</p></body></html>"
            .as_bytes()
            .to_vec();
        mobi.compression = 0x4448;
        let mut database = mobi.database();
        database.records[0][0x70..0x74].copy_from_slice(&2u32.to_be_bytes());
        database.records[1] = vec![255];
        database.records.push(self.huff_record());
        let phrases = if self.recursive {
            vec![
                DictionaryPhrase::new(&[254, 253], false),
                DictionaryPhrase::new(b"<html><body><p>", true),
                DictionaryPhrase::new(
                    "Café 👩‍🔬 Huffman passage.</p></body></html>".as_bytes(),
                    true,
                ),
            ]
        } else {
            vec![DictionaryPhrase::new(&mobi.html, true)]
        };
        for chunk in phrases.chunks(2) {
            let mut dictionary = vec![0u8; 16 + chunk.len() * 2];
            dictionary[..8].copy_from_slice(b"CDIC\0\0\0\x10");
            dictionary[8..12].copy_from_slice(&(phrases.len() as u32).to_be_bytes());
            dictionary[12..16].copy_from_slice(&self.dictionary_bits.to_be_bytes());
            for (index, phrase) in chunk.iter().enumerate() {
                let offset = (dictionary.len() - 16) as u16;
                dictionary[16 + index * 2..18 + index * 2].copy_from_slice(&offset.to_be_bytes());
                let length = phrase.bytes.len() as u16 + if self.truncate_phrase { 100 } else { 0 };
                dictionary.extend((length | if phrase.leaf { 0x8000 } else { 0 }).to_be_bytes());
                dictionary.extend(&phrase.bytes);
            }
            database.records.push(dictionary);
        }
        let count = (database.records.len() - 2) as u32;
        database.records[0][0x74..0x78].copy_from_slice(&count.to_be_bytes());
        if self.trailing_header_bytes > 0 {
            database.records[0][0xf2..0xf4].copy_from_slice(&2u16.to_be_bytes());
            let record = &mut database.records[1];
            record.extend(std::iter::repeat_n(0x55, self.trailing_header_bytes - 1));
            record.push(0x80 | self.trailing_header_bytes as u8);
        }
        database
    }
}

fn strip_fixture_trailer(bytes: &[u8], flags: u16) -> &[u8] {
    let mut end = bytes.len();
    for bit in 1..16 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        let mut size = 0usize;
        for shift in [0, 7, 14, 21] {
            let byte = bytes[end - 1 - shift / 7];
            size |= ((byte & 127) as usize) << shift;
            if byte & 128 != 0 {
                break;
            }
        }
        end -= size;
    }
    if flags & 1 != 0 {
        end -= (bytes[end - 1] & 3) as usize + 1;
    }
    &bytes[..end]
}

fn decode_fixture_palmdoc(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let code = bytes[position];
        position += 1;
        match code {
            0 | 9..=127 => output.push(code),
            1..=8 => {
                output.extend_from_slice(&bytes[position..position + code as usize]);
                position += code as usize;
            }
            128..=191 => {
                let encoded = ((code as usize & 63) << 8) | bytes[position] as usize;
                position += 1;
                let distance = encoded >> 3;
                assert!(
                    distance > 0 && distance <= output.len(),
                    "fixture back-reference distance={distance} output={} position={position} input_len={}",
                    output.len(),
                    bytes.len()
                );
                for _ in 0..(encoded & 7) + 3 {
                    output.push(output[output.len() - distance]);
                }
            }
            _ => {
                output.push(b' ');
                output.push(code ^ 128);
            }
        }
    }
    output
}

pub struct CombinedBuilder {
    pub modern: PalmDatabase,
    pub boundary_marker: Vec<u8>,
    pub boundary_index: Option<u32>,
    pub shared_assets: Vec<Vec<u8>>,
    pub original_language: Option<String>,
}

impl CombinedBuilder {
    pub fn new(modern: PalmDatabase) -> Self {
        Self {
            modern,
            boundary_marker: b"BOUNDARY".to_vec(),
            boundary_index: None,
            shared_assets: Vec::new(),
            original_language: None,
        }
    }

    pub fn database(&self) -> PalmDatabase {
        let modern_index = (3 + self.shared_assets.len()) as u32;
        let mut original = MobiBuilder::new();
        original.title = "Old rendition".into();
        if let Some(language) = &self.original_language {
            original
                .extended
                .push(ExtendedRecord::new(524, language.as_bytes()));
        }
        original.html = b"<p>Old rendition must not replace KF8.</p>".to_vec();
        original.extended.push(ExtendedRecord::new(
            121,
            self.boundary_index.unwrap_or(modern_index).to_be_bytes(),
        ));
        let mut database = original.database();
        if !self.shared_assets.is_empty() {
            database.records[0][0x6c..0x70].copy_from_slice(&2u32.to_be_bytes());
            database.records.extend(self.shared_assets.clone());
        }
        database.records.push(self.boundary_marker.clone());
        database.records.extend(self.modern.records.clone());
        database
    }
}
