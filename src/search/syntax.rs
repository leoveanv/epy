use crate::{Error, Result};

// #NOTE: Search word characters are Unicode letters/numbers plus underscore; combining marks and other connectors are excluded.
const WORD: &str = r"\p{L}\p{N}_";
// #NOTE: Search whitespace includes Unicode whitespace and the four ASCII information separators.
const SPACE: &str = r"\x{9}-\x{D}\x{1C}-\x{20}\x{85}\x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}";

#[derive(Clone, Copy)]
struct Flags {
    insensitive: bool,
    ascii: bool,
    extended: bool,
}

enum Atom {
    Literal(char),
    Set(String),
    Hyphen,
}

struct Parser<'a> {
    input: &'a str,
    position: usize,
    flags: Flags,
    stack: Vec<Flags>,
    global_flags_allowed: bool,
}

fn invalid(message: &str) -> Error {
    Error::Compatibility(format!("Regex error: {message}"))
}
fn code_point(character: char) -> String {
    format!(r"\x{{{:X}}}", character as u32)
}
fn i_equivalent(character: char) -> bool {
    matches!(character, 'i' | 'I' | 'İ' | 'ı')
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.input[self.position..].chars().next()
    }
    fn take(&mut self) -> Option<char> {
        let value = self.peek()?;
        self.position += value.len_utf8();
        Some(value)
    }

    fn character_set(&self, kind: char) -> String {
        let contents = match kind.to_ascii_lowercase() {
            'w' if self.flags.ascii => "A-Za-z0-9_",
            'w' => WORD,
            'd' if self.flags.ascii => "0-9",
            'd' => r"\p{Nd}",
            's' if self.flags.ascii => r"\x{9}-\x{D}\x{20}",
            _ => SPACE,
        };
        format!(
            "[{}{contents}]",
            if kind.is_ascii_uppercase() { "^" } else { "" }
        )
    }

    fn boundary(&self, inverse: bool) -> String {
        let word = self.character_set('w');
        // #NOTE: A word boundary separates a word character from a non-word character or a string endpoint.
        let boundary = format!("(?:(?<!{word})(?={word})|(?<={word})(?!{word}))");
        if inverse {
            // #NOTE: Non-boundaries require a neighboring character, so an empty rendered line does not match.
            format!(r"(?:(?=[\s\S])|(?<=[\s\S]))(?!{boundary})")
        } else {
            boundary
        }
    }

    fn hexadecimal(&mut self, count: usize) -> Result<char> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.input.len() && self.input.is_char_boundary(*end))
            .ok_or_else(|| invalid("incomplete hexadecimal escape"))?;
        let value = u32::from_str_radix(&self.input[self.position..end], 16)
            .map_err(|_| invalid("invalid hexadecimal escape"))?;
        self.position = end;
        char::from_u32(value).ok_or_else(|| invalid("invalid Unicode code point"))
    }

    fn escape(&mut self, in_class: bool) -> Result<Atom> {
        let character = self.take().ok_or_else(|| invalid("trailing backslash"))?;
        Ok(match character {
            'u' => Atom::Literal(self.hexadecimal(4)?),
            'U' => Atom::Literal(self.hexadecimal(8)?),
            'x' => Atom::Literal(self.hexadecimal(2)?),
            'N' => {
                if self.take() != Some('{') {
                    return Err(invalid("a Unicode name must use braces"));
                }
                let start = self.position;
                while self.peek().is_some_and(|value| value != '}') {
                    self.take();
                }
                let name = &self.input[start..self.position];
                if self.take() != Some('}') {
                    return Err(invalid("unterminated Unicode name"));
                }
                Atom::Literal(
                    unicode_names2::character(name)
                        .ok_or_else(|| invalid("unknown Unicode character name"))?,
                )
            }
            'w' | 'W' | 'd' | 'D' | 's' | 'S' => Atom::Set(self.character_set(character)),
            'b' if in_class => Atom::Literal('\u{08}'),
            'b' => Atom::Set(self.boundary(false)),
            'B' if !in_class => Atom::Set(self.boundary(true)),
            'Z' if !in_class => Atom::Set(r"\z".into()),
            'A' if !in_class => Atom::Set(r"\A".into()),
            'a' => Atom::Literal('\u{07}'),
            'f' => Atom::Literal('\u{0c}'),
            'n' => Atom::Literal('\n'),
            'r' => Atom::Literal('\r'),
            't' => Atom::Literal('\t'),
            'v' => Atom::Literal('\u{0b}'),
            '0'..='7' if in_class || character == '0' => {
                let mut digits = character.to_string();
                while digits.len() < 3 && self.peek().is_some_and(|next| matches!(next, '0'..='7'))
                {
                    digits.push(self.take().unwrap_or('0'));
                }
                let value =
                    u32::from_str_radix(&digits, 8).map_err(|_| invalid("invalid octal escape"))?;
                if value > 255 {
                    return Err(invalid("octal escape exceeds one byte"));
                }
                Atom::Literal(char::from_u32(value).ok_or_else(|| invalid("invalid octal escape"))?)
            }
            '1'..='9' if !in_class => Atom::Set(format!("\\{character}")),
            value if value.is_ascii_alphanumeric() => {
                return Err(invalid("unknown escape sequence"));
            }
            value => Atom::Literal(value),
        })
    }

    fn literal(&self, character: char) -> String {
        if self.flags.insensitive && !self.flags.ascii && i_equivalent(character) {
            "[iİı]".into()
        } else {
            code_point(character)
        }
    }

    fn class(&mut self) -> Result<String> {
        let negate = self.peek() == Some('^');
        if negate {
            self.take();
        }
        let mut atoms = Vec::new();
        loop {
            let character = self
                .take()
                .ok_or_else(|| invalid("unterminated character class"))?;
            match character {
                ']' if !atoms.is_empty() => break,
                '\\' => atoms.push(self.escape(true)?),
                '-' => atoms.push(Atom::Hyphen),
                value => atoms.push(Atom::Literal(value)),
            }
        }
        if matches!(atoms.first(), Some(Atom::Hyphen)) {
            atoms[0] = Atom::Literal('-');
        }
        let last = atoms.len().saturating_sub(1);
        if matches!(atoms.last(), Some(Atom::Hyphen)) {
            atoms[last] = Atom::Literal('-');
        }
        let mut base = String::new();
        let mut sets = Vec::new();
        let mut includes_i = false;
        for (index, atom) in atoms.iter().enumerate() {
            match atom {
                Atom::Literal(character) => {
                    base.push_str(&code_point(*character));
                    includes_i |= i_equivalent(*character);
                }
                Atom::Set(set) => sets.push(set.clone()),
                Atom::Hyphen if index == 0 || index + 1 == atoms.len() => base.push_str(r"\x{2D}"),
                Atom::Hyphen => {
                    let (Atom::Literal(start), Atom::Literal(end)) =
                        (&atoms[index - 1], &atoms[index + 1])
                    else {
                        return Err(invalid(
                            "character range endpoints must be literal characters",
                        ));
                    };
                    if start > end {
                        return Err(invalid("character range is reversed"));
                    }
                    includes_i |= (*start <= 'I' && 'I' <= *end) || (*start <= 'i' && 'i' <= *end);
                    base.push('-');
                }
            }
        }
        if includes_i && self.flags.insensitive && !self.flags.ascii {
            base.push_str("İıIi");
        }
        if !base.is_empty() {
            sets.push(format!("[{base}]"));
        }
        // #NOTE: A Perl extended class unions the original class atoms without changing their single-character width.
        Ok(format!(
            "(?[{}({})])",
            if negate { "!" } else { "" },
            sets.join(" | ")
        ))
    }

    fn copy_header(&mut self, terminator: char) -> Result<String> {
        let start = self.position;
        while let Some(character) = self.take() {
            if character == terminator {
                return Ok(self.input[start..self.position].into());
            }
        }
        Err(invalid("unterminated group header"))
    }

    fn flags(&mut self) -> Result<Option<String>> {
        let Some(remaining) = self.input.get(self.position + 2..) else {
            return Ok(None);
        };
        let Some(end) = remaining.find([':', ')']) else {
            return Ok(None);
        };
        let source = &remaining[..end];
        if source.is_empty()
            || !source
                .chars()
                .all(|character| "aimsux-".contains(character))
        {
            return Ok(None);
        }
        let scoped = remaining.as_bytes()[end] == b':';
        if !scoped && !self.global_flags_allowed {
            return Err(invalid(
                "global flags must appear at the start of the expression",
            ));
        }
        if scoped {
            self.global_flags_allowed = false;
            self.stack.push(self.flags);
        }
        let mut negative = false;
        let mut positive = String::new();
        let mut removed = String::new();
        for flag in source.chars() {
            match flag {
                '-' => negative = true,
                'i' => {
                    self.flags.insensitive = !negative;
                    if negative {
                        removed.push(flag);
                    } else {
                        positive.push(flag);
                    }
                }
                'x' => {
                    self.flags.extended = !negative;
                    if negative {
                        removed.push(flag);
                    } else {
                        positive.push(flag);
                    }
                }
                'a' | 'u' if negative => {
                    return Err(invalid("ASCII and Unicode flags cannot be disabled"));
                }
                'a' => {
                    self.flags.ascii = true;
                    positive.push_str("ar");
                }
                'u' => {
                    self.flags.ascii = false;
                    removed.push_str("ar");
                }
                value => {
                    if negative {
                        removed.push(value);
                    } else {
                        positive.push(value);
                    }
                }
            }
        }
        self.position += 2 + end + 1;
        Ok(Some(format!(
            "(?{positive}{}{}{}",
            if removed.is_empty() { "" } else { "-" },
            removed,
            if scoped { ":" } else { ")" }
        )))
    }

    fn group(&mut self) -> Result<String> {
        let input = self.input;
        let remaining = &input[self.position..];
        if remaining.starts_with("(?#") {
            return self.copy_header(')');
        }
        if remaining.starts_with("(?P=") {
            self.global_flags_allowed = false;
            return self.copy_header(')');
        }
        if let Some(flags) = self.flags()? {
            return Ok(flags);
        }
        self.global_flags_allowed = false;
        self.stack.push(self.flags);
        if remaining.starts_with("(?P<")
            || (remaining.starts_with("(?<")
                && !remaining.starts_with("(?<=")
                && !remaining.starts_with("(?<!"))
        {
            return self.copy_header('>');
        }
        if remaining.starts_with("(?'") {
            self.position += 3;
            return Ok(format!("(?'{}", self.copy_header('\'')?));
        }
        if remaining.starts_with("(?(") {
            return self.copy_header(')');
        }
        let count = if remaining.starts_with("(?<=") || remaining.starts_with("(?<!") {
            4
        } else if ["(?:", "(?=", "(?!", "(?>"]
            .iter()
            .any(|prefix| remaining.starts_with(prefix))
        {
            3
        } else {
            1
        };
        self.position += count;
        Ok(remaining[..count].into())
    }

    fn prepare(&mut self) -> Result<String> {
        let mut output = String::new();
        while let Some(character) = self.peek() {
            match character {
                '\\' => {
                    self.global_flags_allowed = false;
                    self.take();
                    match self.escape(false)? {
                        Atom::Literal(value) => output.push_str(&self.literal(value)),
                        Atom::Set(code) => output.push_str(&code),
                        Atom::Hyphen => {}
                    }
                }
                '[' => {
                    self.global_flags_allowed = false;
                    self.take();
                    output.push_str(&self.class()?);
                }
                '(' => output.push_str(&self.group()?),
                ')' => {
                    self.take();
                    if let Some(flags) = self.stack.pop() {
                        self.flags = flags;
                    }
                    output.push(')');
                }
                '#' if self.flags.extended => {
                    let start = self.position;
                    while self.take().is_some_and(|value| value != '\n') {}
                    output.push_str(&self.input[start..self.position]);
                }
                value => {
                    if !self.flags.extended || !value.is_whitespace() {
                        self.global_flags_allowed = false;
                    }
                    self.take();
                    if self.flags.insensitive && !self.flags.ascii && i_equivalent(value) {
                        output.push_str("[iİı]");
                    } else {
                        output.push(value);
                    }
                }
            }
            if output.len() > 1024 * 1024 {
                return Err(invalid("expanded pattern exceeds its size limit"));
            }
        }
        Ok(output)
    }
}

pub(super) fn prepare(pattern: &str) -> Result<String> {
    Parser {
        input: pattern,
        position: 0,
        flags: Flags {
            insensitive: true,
            ascii: false,
            extended: false,
        },
        stack: Vec::new(),
        global_flags_allowed: true,
    }
    .prepare()
}
