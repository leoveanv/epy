use std::collections::VecDeque;
use std::sync::LazyLock;

use serde::Deserialize;
use unicode_segmentation::UnicodeSegmentation;

use super::{normalize_paragraph, python_whitespace};

#[derive(Deserialize)]
struct CodePointRange {
    start: u32,
    end: u32,
}

#[derive(Deserialize)]
struct WidthTable {
    zero: Vec<CodePointRange>,
    wide: Vec<CodePointRange>,
    variation: Vec<CodePointRange>,
}

// #NOTE: These MIT-licensed wcwidth 0.2.14 tables preserve the Python baseline's Unicode 17 widths. See WCWIDTH-LICENSE.
static WIDTHS: LazyLock<WidthTable> = LazyLock::new(|| {
    serde_json::from_str(include_str!("width_ranges.json"))
        .expect("The embedded terminal-width ranges are valid JSON")
});

fn in_ranges(character: char, ranges: &[CodePointRange]) -> bool {
    let code_point = character as u32;
    ranges
        .binary_search_by(|range| {
            if code_point < range.start {
                std::cmp::Ordering::Greater
            } else if code_point > range.end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

pub fn graphemes(value: &str) -> Vec<&str> {
    value.graphemes(true).collect()
}

// #NOTE: Terminal width calculation measures ordinary characters separately, skips ZWJ continuations, and widens selected VS16 sequences.
pub fn cell_width(value: &str) -> usize {
    if value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return value.len();
    }
    let mut width = 0;
    let mut characters = value.chars();
    let mut last_measured = None;
    while let Some(character) = characters.next() {
        if character == '\u{200d}' {
            characters.next();
            continue;
        }
        if character == '\u{fe0f}' && last_measured.is_some() {
            let previous = last_measured.take().unwrap_or('\0');
            width += usize::from(in_ranges(previous, &WIDTHS.variation));
            continue;
        }
        if matches!(character, '\u{01}'..='\u{1f}' | '\u{7f}'..='\u{9f}') {
            return 0;
        }
        let occupied = if in_ranges(character, &WIDTHS.zero) {
            0
        } else {
            1 + usize::from(in_ranges(character, &WIDTHS.wide))
        };
        if occupied > 0 {
            last_measured = Some(character);
        }
        width += occupied;
    }
    width
}

pub fn clip_text(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut occupied = 0;
    let mut end = 0;
    for (index, cluster) in value.grapheme_indices(true) {
        occupied += cell_width(cluster);
        if occupied > width {
            break;
        }
        end = index + cluster.len();
    }
    value[..end].into()
}

pub fn wrap_text(value: &str, width: usize) -> Vec<String> {
    if value.is_ascii() {
        wrap_ascii(value, width.max(1))
    } else {
        wrap_unicode(value, width.max(2))
    }
}

fn wrap_unicode(value: &str, width: usize) -> Vec<String> {
    let expanded = normalize_paragraph(value).replace('\n', " ");
    let mut remaining = expanded.trim_matches(python_whitespace);
    let mut lines = Vec::new();
    while !remaining.is_empty() {
        let mut fragment = clip_text(remaining, width);
        if fragment.is_empty() {
            break;
        }
        if fragment.len() < remaining.len()
            && !remaining[fragment.len()..].starts_with(python_whitespace)
            && let Some(boundary) = fragment.rfind(' ').filter(|boundary| *boundary > 0)
        {
            fragment.truncate(boundary);
        }
        lines.push(fragment.trim_end_matches(python_whitespace).into());
        remaining = remaining[fragment.len()..].trim_start_matches(python_whitespace);
    }
    lines
}

fn word_letter(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn word_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn word_punctuation(byte: u8) -> bool {
    word_character(byte) || matches!(byte, b'!' | b'"' | b'\'' | b'&' | b'.' | b',' | b'?')
}

fn hyphen_boundary(bytes: &[u8], index: usize) -> bool {
    let left = (index >= 2 && word_letter(bytes[index - 1]) && word_letter(bytes[index - 2]))
        || (index >= 3
            && word_letter(bytes[index - 1])
            && bytes[index - 2] == b'-'
            && word_letter(bytes[index - 3]));
    let right = bytes.get(index + 1).is_some_and(|byte| word_letter(*byte))
        && (bytes.get(index + 2).is_some_and(|byte| word_letter(*byte))
            || (bytes.get(index + 2) == Some(&b'-')
                && bytes.get(index + 3).is_some_and(|byte| word_letter(*byte))));
    left && right
}

fn split_word<'a>(word: &'a str, chunks: &mut VecDeque<&'a str>) {
    let bytes = word.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'-' {
            index += 1;
            continue;
        }
        let dashes = bytes[index..]
            .iter()
            .take_while(|byte| **byte == b'-')
            .count();
        if dashes >= 2
            && index > 0
            && word_punctuation(bytes[index - 1])
            && bytes
                .get(index + dashes)
                .is_some_and(|byte| word_character(*byte))
        {
            if start < index {
                chunks.push_back(&word[start..index]);
            }
            chunks.push_back(&word[index..index + dashes]);
            index += dashes;
            start = index;
        } else {
            if hyphen_boundary(bytes, index) {
                chunks.push_back(&word[start..=index]);
                start = index + 1;
            }
            index += 1;
        }
    }
    if start < word.len() {
        chunks.push_back(&word[start..]);
    }
}

fn ascii_chunks(value: &str) -> VecDeque<&str> {
    let mut chunks = VecDeque::new();
    let mut remaining = value;
    while !remaining.is_empty() {
        let whitespace = remaining.starts_with(' ');
        let length = remaining
            .bytes()
            .take_while(|byte| (*byte == b' ') == whitespace)
            .count();
        let fragment = &remaining[..length];
        if whitespace {
            chunks.push_back(fragment);
        } else {
            split_word(fragment, &mut chunks);
        }
        remaining = &remaining[length..];
    }
    chunks
}

fn append_long_chunk<'a>(
    chunks: &mut VecDeque<&'a str>,
    pieces: &mut Vec<&'a str>,
    available: usize,
) {
    let Some(chunk) = chunks.pop_front() else {
        return;
    };
    let mut end = available.min(chunk.len());
    if let Some(hyphen) = chunk[..end].rfind('-').filter(|index| *index > 0)
        && chunk[..hyphen].bytes().any(|byte| byte != b'-')
    {
        end = hyphen + 1;
    }
    pieces.push(&chunk[..end]);
    if end < chunk.len() {
        chunks.push_front(&chunk[end..]);
    }
}

fn wrap_ascii(value: &str, width: usize) -> Vec<String> {
    let expanded = normalize_paragraph(value).replace('\n', " ");
    let mut chunks = ascii_chunks(&expanded);
    let mut lines = Vec::new();
    while !chunks.is_empty() {
        if !lines.is_empty()
            && chunks
                .front()
                .is_some_and(|chunk| chunk.trim_matches(python_whitespace).is_empty())
        {
            chunks.pop_front();
        }
        let mut pieces = Vec::new();
        let mut occupied = 0;
        while let Some(chunk) = chunks.front().copied() {
            if chunk.len() > width - occupied {
                break;
            }
            occupied += chunk.len();
            pieces.push(chunk);
            chunks.pop_front();
        }
        if chunks.front().is_some_and(|chunk| chunk.len() > width) {
            append_long_chunk(&mut chunks, &mut pieces, width - occupied);
        }
        if pieces
            .last()
            .is_some_and(|piece| piece.trim_matches(python_whitespace).is_empty())
        {
            pieces.pop();
        }
        if !pieces.is_empty() {
            lines.push(pieces.concat());
        }
    }
    lines
}
