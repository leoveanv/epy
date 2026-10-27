use super::unescape_html;

#[derive(Debug)]
pub(crate) struct HtmlAttribute {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug)]
pub(crate) struct HtmlTag {
    pub name: String,
    pub attributes: Vec<HtmlAttribute>,
    pub self_closing: bool,
}

pub(crate) enum HtmlToken {
    Start(HtmlTag),
    End(String),
    Data(String),
}

pub(crate) struct HtmlTokenizer<'a> {
    remaining: &'a str,
    raw_tag: Option<String>,
    pending_end: Option<String>,
}

impl<'a> HtmlTokenizer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            remaining: source,
            raw_tag: None,
            pending_end: None,
        }
    }

    fn skip_markup(&mut self, prefix: &str, endings: &[&str]) {
        let ending = endings
            .iter()
            .filter_map(|ending| {
                self.remaining[prefix.len()..]
                    .find(ending)
                    .map(|index| index + prefix.len() + ending.len())
            })
            .min();
        self.remaining = ending.map_or("", |end| &self.remaining[end..]);
    }

    fn raw_end(&mut self) -> Option<HtmlToken> {
        let tag = self.raw_tag.take()?;
        let marker = format!("</{tag}");
        let position = self
            .remaining
            .as_bytes()
            .windows(marker.len())
            .enumerate()
            .find_map(|(index, window)| {
                if !window.eq_ignore_ascii_case(marker.as_bytes()) {
                    return None;
                }
                let tail = &self.remaining[index + marker.len()..];
                let whitespace = tail.bytes().take_while(|byte| html_space(*byte)).count();
                tail[whitespace..]
                    .starts_with('>')
                    .then_some((index, index + marker.len() + whitespace + 1))
            });
        let (start, end) = match position {
            Some(position) => position,
            None => {
                self.remaining = "";
                return None;
            }
        };
        let raw = &self.remaining[..start];
        let data = if matches!(tag.as_str(), "title" | "textarea") {
            unescape_html(raw)
        } else {
            raw.into()
        };
        self.remaining = &self.remaining[end..];
        if data.is_empty() {
            Some(HtmlToken::End(tag))
        } else {
            self.pending_end = Some(tag);
            Some(HtmlToken::Data(data))
        }
    }
}

impl Iterator for HtmlTokenizer<'_> {
    type Item = HtmlToken;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(tag) = self.pending_end.take() {
            return Some(HtmlToken::End(tag));
        }
        if self.raw_tag.is_some() {
            return self.raw_end();
        }
        while !self.remaining.is_empty() {
            let index = self.remaining.find('<').unwrap_or(self.remaining.len());
            if index != 0 {
                let data = unescape_html(&self.remaining[..index]);
                self.remaining = &self.remaining[index..];
                return Some(HtmlToken::Data(data));
            }
            if self.remaining.starts_with("<!--") {
                self.skip_markup("<!--", &["-->", "--!>"]);
                continue;
            }
            if self.remaining.starts_with("<![CDATA[") {
                self.skip_markup("<![CDATA[", &["]]>"]);
                continue;
            }
            if self.remaining.starts_with("<!") || self.remaining.starts_with("<?") {
                self.skip_markup("<!", &[">"]);
                continue;
            }
            let is_end = self.remaining.starts_with("</");
            let offset = if is_end { 2 } else { 1 };
            if self
                .remaining
                .as_bytes()
                .get(offset)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                let Some(parsed) = parse_tag(self.remaining, offset) else {
                    self.remaining = "";
                    return None;
                };
                self.remaining = &self.remaining[parsed.consumed..];
                if is_end {
                    return Some(HtmlToken::End(parsed.tag.name));
                }
                if !parsed.tag.self_closing
                    && matches!(
                        parsed.tag.name.as_str(),
                        "script" | "style" | "title" | "textarea"
                    )
                {
                    self.raw_tag = Some(parsed.tag.name.clone());
                }
                return Some(HtmlToken::Start(parsed.tag));
            }
            if is_end {
                self.skip_markup("</", &[">"]);
                continue;
            }
            self.remaining = &self.remaining[1..];
            return Some(HtmlToken::Data("<".into()));
        }
        None
    }
}

fn html_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

fn skip_separators(source: &str, mut cursor: usize) -> usize {
    while let Some(byte) = source.as_bytes().get(cursor) {
        if html_space(*byte) || (*byte == b'/' && source.as_bytes().get(cursor + 1) != Some(&b'>'))
        {
            cursor += 1;
        } else {
            break;
        }
    }
    cursor
}

struct ParsedTag {
    tag: HtmlTag,
    consumed: usize,
}

fn parse_tag(source: &str, offset: usize) -> Option<ParsedTag> {
    let name_end = offset
        + source[offset..]
            .bytes()
            .take_while(|byte| !html_space(*byte) && !matches!(*byte, b'/' | b'>'))
            .count();
    let mut tag = HtmlTag {
        name: source[offset..name_end].to_lowercase(),
        attributes: Vec::new(),
        self_closing: false,
    };
    let mut cursor = skip_separators(source, name_end);
    while cursor < source.len() {
        if source[cursor..].starts_with("/>") {
            tag.self_closing = true;
            return Some(ParsedTag {
                tag,
                consumed: cursor + 2,
            });
        }
        if source.as_bytes()[cursor] == b'>' {
            return Some(ParsedTag {
                tag,
                consumed: cursor + 1,
            });
        }
        let attribute = parse_attribute(source, cursor)?;
        tag.attributes.push(attribute.attribute);
        cursor = skip_separators(source, attribute.consumed);
    }
    None
}

struct ParsedAttribute {
    attribute: HtmlAttribute,
    consumed: usize,
}

fn parse_attribute(source: &str, cursor: usize) -> Option<ParsedAttribute> {
    let first = source[cursor..].chars().next()?;
    let name_end = cursor
        + first.len_utf8()
        + source[cursor + first.len_utf8()..]
            .bytes()
            .take_while(|byte| !html_space(*byte) && !matches!(*byte, b'/' | b'=' | b'>'))
            .count();
    let name = source[cursor..name_end].to_lowercase();
    let mut cursor = name_end;
    cursor += source[cursor..]
        .bytes()
        .take_while(|byte| html_space(*byte))
        .count();
    let value = if source.as_bytes().get(cursor) == Some(&b'=') {
        cursor += 1;
        cursor += source[cursor..]
            .bytes()
            .take_while(|byte| html_space(*byte))
            .count();
        let quote = source.as_bytes().get(cursor).copied()?;
        let start;
        let end;
        if matches!(quote, b'\'' | b'"') {
            start = cursor + 1;
            end = start + source[start..].find(quote as char)?;
            cursor = end + 1;
        } else {
            start = cursor;
            end = start
                + source[start..]
                    .bytes()
                    .take_while(|byte| !html_space(*byte) && *byte != b'>')
                    .count();
            cursor = end;
        }
        Some(unescape_html(&source[start..end]))
    } else {
        None
    };
    Some(ParsedAttribute {
        attribute: HtmlAttribute { name, value },
        consumed: cursor,
    })
}
