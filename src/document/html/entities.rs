use std::sync::LazyLock;

static LEGACY_ENTITIES: LazyLock<Vec<&str>> = LazyLock::new(|| {
    include_str!("entity_shortforms.txt")
        .split_whitespace()
        .collect()
});

// #NOTE: Python permits legacy names without semicolons and decodes numeric references using HTML5 replacement rules.
pub(crate) fn unescape_html(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(index) = remaining.find('&') {
        output.push_str(&remaining[..index]);
        remaining = &remaining[index + 1..];
        if let Some(decoded) = decode_reference(remaining) {
            output.push_str(&decoded.text);
            remaining = &remaining[decoded.consumed..];
        } else {
            output.push('&');
        }
    }
    output.push_str(remaining);
    output
}

struct DecodedReference {
    text: String,
    consumed: usize,
}

fn decode_reference(value: &str) -> Option<DecodedReference> {
    if let Some(numeric) = value.strip_prefix('#') {
        return decode_numeric_reference(numeric);
    }
    let mut end = 0;
    for character in value.chars().take(32) {
        if matches!(
            character,
            '\t' | '\n' | '\u{000c}' | ' ' | '<' | '&' | '#' | ';'
        ) {
            break;
        }
        end += character.len_utf8();
    }
    if end == 0 {
        return None;
    }
    if value[end..].starts_with(';') {
        end += 1;
    }
    let name = &value[..end];
    for (index, character) in name.char_indices().rev() {
        let prefix_end = index + character.len_utf8();
        let candidate = &name[..prefix_end];
        let key = if let Some(key) = candidate.strip_suffix(';') {
            key
        } else if LEGACY_ENTITIES.binary_search(&candidate).is_ok() {
            candidate
        } else {
            continue;
        };
        if let Ok(index) =
            html_escape::NAMED_ENTITIES.binary_search_by(|(name, _)| name.cmp(&key.as_bytes()))
        {
            return Some(DecodedReference {
                text: html_escape::NAMED_ENTITIES[index].1.into(),
                consumed: prefix_end,
            });
        }
    }
    None
}

fn decode_numeric_reference(value: &str) -> Option<DecodedReference> {
    let hexadecimal = value.starts_with(['x', 'X']);
    let offset = usize::from(hexadecimal);
    let digits = &value[offset..];
    let count = digits
        .bytes()
        .take_while(|byte| {
            if hexadecimal {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }
        })
        .count();
    if count == 0 {
        return None;
    }
    let number = u32::from_str_radix(&digits[..count], if hexadecimal { 16 } else { 10 })
        .unwrap_or(0x110000);
    let consumed = 1 + offset + count + usize::from(digits[count..].starts_with(';'));
    Some(DecodedReference {
        text: numeric_character(number),
        consumed,
    })
}

fn numeric_character(number: u32) -> String {
    if number == 0 || number > 0x10ffff || (0xd800..=0xdfff).contains(&number) {
        return "\u{fffd}".into();
    }
    if (0x80..=0x9f).contains(&number) {
        return "€\u{81}‚ƒ„…†‡ˆ‰Š‹Œ\u{8d}Ž\u{8f}\u{90}‘’“”•–—˜™š›œ\u{9d}žŸ"
            .chars()
            .nth((number - 0x80) as usize)
            .unwrap_or('\u{fffd}')
            .to_string();
    }
    if matches!(number, 1..=8 | 0xb | 0xe..=0x1f | 0x7f | 0xfdd0..=0xfdef)
        || (number & 0xffff) >= 0xfffe
    {
        return String::new();
    }
    char::from_u32(number).unwrap_or('\u{fffd}').to_string()
}
