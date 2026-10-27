mod references;
mod wrapping;
use crate::document::DocumentReferences;
use references::ReferenceMetadata;

pub use wrapping::{cell_width, clip_text, graphemes, wrap_text};

use percent_encoding::percent_decode_str;

use crate::document::SourceDocument;
use crate::document::html::unescape_html;
use crate::document::html::{HtmlTag, HtmlToken, HtmlTokenizer};
use crate::document::layout::{FontStyle, ParagraphStyle};
use crate::document::links::{CharacterPosition, DocumentSemantics, LinkMetadata, TextMark};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParagraphRole {
    #[default]
    Body,
    Heading,
    Indented,
    Bullet,
    Preformatted,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paragraph {
    pub text: String,
    pub raw_text: String,
    pub source_start: usize,
    pub role: ParagraphRole,
    pub image_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedDocument {
    pub source: SourceDocument,
    pub paragraphs: Vec<Paragraph>,
    pub semantics: DocumentSemantics,
    pub references: DocumentReferences,
    pub(crate) styles: Vec<ParagraphStyle>,
    pub(crate) suppressed_images: Vec<usize>,
}

#[derive(Default)]
struct ParagraphFlags {
    heading: bool,
    indented: bool,
    bullet: bool,
    preformatted: bool,
}

#[derive(Default)]
struct RawParagraph {
    text: String,
    flags: ParagraphFlags,
    image_path: Option<String>,
    suppress_image: bool,
}

#[derive(PartialEq, Eq)]
enum ImagePlatform {
    Epub,
    Amazon,
}

// #NOTE: Adjacent EPUB/Amazon variants of the same asset retain both source markers but share one displayed image.
struct ImageVariant {
    group: String,
    platform: ImagePlatform,
    path: String,
}

impl ImageVariant {
    fn from_tag(tag: &HtmlTag, path: &str) -> Option<Self> {
        let classes = tag
            .attributes
            .iter()
            .find(|attribute| attribute.name == "class")?
            .value
            .as_deref()?;
        classes.split_whitespace().find_map(|class| {
            let (group, platform) = if let Some(group) = class.strip_suffix("-epub") {
                (group, ImagePlatform::Epub)
            } else if let Some(group) = class.strip_suffix("-amzn") {
                (group, ImagePlatform::Amazon)
            } else {
                return None;
            };
            Some(Self {
                group: group.into(),
                platform,
                path: path.into(),
            })
        })
    }

    fn is_alternative_to(&self, previous: &Self) -> bool {
        self.group == previous.group
            && self.path == previous.path
            && self.platform != previous.platform
    }
}

impl RawParagraph {
    fn role(&self) -> ParagraphRole {
        if self.flags.heading {
            ParagraphRole::Heading
        } else if self.flags.indented {
            ParagraphRole::Indented
        } else if self.flags.bullet {
            ParagraphRole::Bullet
        } else if self.flags.preformatted {
            ParagraphRole::Preformatted
        } else if self.image_path.is_some() {
            ParagraphRole::Image
        } else {
            ParagraphRole::Body
        }
    }
}

struct DocumentParser {
    paragraphs: Vec<RawParagraph>,
    flags: ParagraphFlags,
    hidden: bool,
    metadata: LinkMetadata,
    references: ReferenceMetadata,
    italics: Vec<TextMark>,
    bold: Vec<TextMark>,
    image_variant: Option<ImageVariant>,
}

impl DocumentParser {
    fn new() -> Self {
        Self {
            paragraphs: vec![RawParagraph::default()],
            flags: ParagraphFlags::default(),
            hidden: false,
            metadata: LinkMetadata::default(),
            references: ReferenceMetadata::default(),
            italics: Vec::new(),
            bold: Vec::new(),
            image_variant: None,
        }
    }

    fn current(&mut self) -> &mut RawParagraph {
        self.paragraphs
            .last_mut()
            .expect("The parser always retains its first paragraph")
    }

    fn blank(&mut self) {
        self.paragraphs.push(RawParagraph::default());
    }

    fn position(&self) -> CharacterPosition {
        CharacterPosition {
            row: self.paragraphs.len() - 1,
            col: self
                .paragraphs
                .last()
                .map_or(0, |paragraph| paragraph.text.chars().count()),
        }
    }

    fn start(&mut self, tag: &HtmlTag) -> Result<()> {
        if !matches!(tag.name.as_str(), "img" | "image") {
            self.image_variant = None;
        }
        let position = self.position();
        if !self.hidden && !matches!(tag.name.as_str(), "head" | "script" | "style") {
            self.references.start(tag, position)?;
            self.metadata.start(tag, position);
            if tag.self_closing {
                self.metadata.end(&tag.name, position);
            }
        }
        if tag.self_closing {
            if tag.name == "br" {
                self.blank();
            } else if matches!(tag.name.as_str(), "img" | "image") {
                self.images(tag, true)?;
            }
            return Ok(());
        }
        if is_heading(&tag.name) {
            self.flags.heading = true;
        } else {
            match tag.name.as_str() {
                "q" | "dt" | "dd" | "blockquote" => self.flags.indented = true,
                "pre" => self.flags.preformatted = true,
                "li" => self.flags.bullet = true,
                "script" | "style" | "head" => self.hidden = true,
                "sup" => self.current().text.push_str("^{"),
                "sub" => self.current().text.push_str("_{"),
                "img" | "image" => self.images(tag, false)?,
                "i" | "em" => start_style(&mut self.italics, position),
                "b" | "strong" => start_style(&mut self.bold, position),
                _ => {}
            }
        }
        Ok(())
    }

    fn images(&mut self, tag: &HtmlTag, self_closing: bool) -> Result<()> {
        let previous = self.image_variant.take();
        for attribute in &tag.attributes {
            if (tag.name == "img" && attribute.name == "src")
                || (tag.name == "image" && attribute.name.ends_with("href"))
            {
                let source = attribute.value.as_deref().ok_or_else(|| {
                    Error::Compatibility("An image source attribute has no value.".into())
                })?;
                let path = percent_decode_str(source).decode_utf8_lossy().into_owned();
                let variant = ImageVariant::from_tag(tag, &path);
                let suppress_image = variant.as_ref().is_some_and(|variant| {
                    previous
                        .as_ref()
                        .is_some_and(|previous| variant.is_alternative_to(previous))
                });
                self.image_variant = if suppress_image { None } else { variant };
                self.paragraphs.push(RawParagraph {
                    text: "[IMAGE]".into(),
                    image_path: Some(path),
                    flags: ParagraphFlags::default(),
                    suppress_image,
                });
                if self_closing {
                    self.blank();
                }
            }
        }
        Ok(())
    }

    fn end(&mut self, tag: &str) {
        if !matches!(tag, "img" | "image") {
            self.image_variant = None;
        }
        if !self.hidden {
            self.references.end(tag, &self.paragraphs);
        }
        if is_heading(tag) {
            self.blank();
            self.blank();
            self.flags.heading = false;
        } else {
            match tag {
                "p" | "div" | "img" | "image" => self.blank(),
                "script" | "style" | "head" => self.hidden = false,
                "q" | "dt" | "dd" | "blockquote" => {
                    if !self.current().text.is_empty() {
                        self.blank();
                    }
                    self.flags.indented = false;
                }
                "pre" => {
                    if !self.current().text.is_empty() {
                        self.blank();
                    }
                    self.flags.preformatted = false;
                }
                "li" => {
                    if !self.current().text.is_empty() {
                        self.blank();
                    }
                    self.flags.bullet = false;
                }
                "sub" | "sup" => self.current().text.push('}'),
                "i" | "em" => {
                    let position = self.position();
                    end_style(&mut self.italics, position);
                }
                "b" | "strong" => {
                    let position = self.position();
                    end_style(&mut self.bold, position);
                }
                _ => {}
            }
        }
        if !self.hidden {
            self.metadata.end(tag, self.position());
        }
    }

    fn data(&mut self, raw: &str) {
        if raw.chars().any(|character| !python_whitespace(character)) {
            self.image_variant = None;
        }
        if raw.is_empty() || self.hidden {
            return;
        }
        let raw = if self.current().text.is_empty() {
            raw.trim_start_matches(python_whitespace)
        } else {
            raw
        };
        let line = if self.flags.preformatted {
            unescape_html(raw)
        } else {
            unescape_html(&collapse_whitespace(raw))
        };
        let flags = ParagraphFlags {
            heading: self.flags.heading,
            bullet: !self.flags.heading && self.flags.bullet,
            indented: !self.flags.heading && !self.flags.bullet && self.flags.indented,
            preformatted: !self.flags.heading
                && !self.flags.bullet
                && !self.flags.indented
                && self.flags.preformatted,
        };
        let paragraph = self.current();
        paragraph.text.push_str(&line);
        paragraph.flags.heading |= flags.heading;
        paragraph.flags.bullet |= flags.bullet;
        paragraph.flags.indented |= flags.indented;
        paragraph.flags.preformatted |= flags.preformatted;
    }

    fn finish(self, content_id: &str) -> ParsedDocument {
        let mut source_start = 0;
        let mut suppressed_images = Vec::new();
        let paragraphs = self
            .paragraphs
            .into_iter()
            .enumerate()
            .map(|(index, raw)| {
                if raw.suppress_image {
                    suppressed_images.push(index);
                }
                let role = raw.role();
                let text = normalize_paragraph(&raw.text);
                let paragraph = Paragraph {
                    text,
                    raw_text: raw.text,
                    source_start,
                    role,
                    image_path: raw.image_path,
                };
                source_start += paragraph.text.chars().count() + 1;
                paragraph
            })
            .collect::<Vec<_>>();
        let text = paragraphs
            .iter()
            .map(|paragraph| paragraph.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let source = SourceDocument::new(content_id, text);
        let semantics = self.metadata.finish(&paragraphs, &source);
        let references = self.references.finish(&paragraphs);
        let mut styles = collect_styles(&paragraphs, &self.italics, FontStyle::Italic);
        styles.extend(collect_styles(&paragraphs, &self.bold, FontStyle::Bold));
        ParsedDocument {
            source,
            paragraphs,
            semantics,
            references,
            styles,
            suppressed_images,
        }
    }
}

fn start_style(marks: &mut Vec<TextMark>, position: CharacterPosition) {
    if marks.last().is_none_or(|mark| mark.is_valid()) {
        marks.push(TextMark {
            start: position,
            end: None,
        });
    }
}

fn end_style(marks: &mut [TextMark], position: CharacterPosition) {
    if let Some(mark) = marks.last_mut() {
        mark.end = Some(position);
    }
}

fn collect_styles(
    paragraphs: &[Paragraph],
    marks: &[TextMark],
    style: FontStyle,
) -> Vec<ParagraphStyle> {
    let mut spans = Vec::new();
    for mark in marks.iter().filter(|mark| mark.is_valid()) {
        let Some(end) = mark.end else {
            continue;
        };
        for (row, paragraph) in paragraphs
            .iter()
            .enumerate()
            .take(end.row + 1)
            .skip(mark.start.row)
        {
            let start = if row == mark.start.row {
                mark.start.col
            } else {
                0
            };
            let end = if row == end.row {
                end.col
            } else {
                paragraph.raw_text.chars().count()
            };
            spans.push(ParagraphStyle {
                row,
                col: start,
                n_letters: end.saturating_sub(start),
                style,
            });
        }
    }
    spans
}

fn is_heading(tag: &str) -> bool {
    tag.as_bytes().first() == Some(&b'h')
        && tag
            .as_bytes()
            .get(1)
            .is_some_and(|byte| (b'1'..=b'6').contains(byte))
}

pub(crate) fn python_whitespace(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{001c}'..='\u{001f}')
}

fn collapse_whitespace(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut previous_whitespace = false;
    for character in value.chars() {
        let whitespace = python_whitespace(character);
        if !whitespace {
            result.push(character);
        } else if !previous_whitespace {
            result.push(' ');
        }
        previous_whitespace = whitespace;
    }
    result
}

pub fn normalize_paragraph(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut column = 0;
    for character in value.chars() {
        match character {
            '\t' => {
                let spaces = 8 - column % 8;
                result.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            '\n' => {
                result.push('\n');
                column = 0;
            }
            '\r' => {
                result.push(' ');
                column = 0;
            }
            '\u{000b}' | '\u{000c}' => {
                result.push(' ');
                column += 1;
            }
            _ => {
                result.push(character);
                column += 1;
            }
        }
    }
    result
}

// #NOTE: Canonical anchors retain tokenizer events, duplicate attributes, CR preservation, and the reader's second entity-decoding pass.
pub fn parse_document(content_id: &str, html: &str) -> Result<ParsedDocument> {
    let mut parser = DocumentParser::new();
    for token in HtmlTokenizer::new(html) {
        match token {
            HtmlToken::Start(tag) => parser.start(&tag)?,
            HtmlToken::End(tag) => parser.end(&tag),
            HtmlToken::Data(data) => parser.data(&data),
        }
    }
    Ok(parser.finish(content_id))
}
