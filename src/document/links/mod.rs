use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::document::SourceDocument;
use crate::document::html::HtmlTag;
use crate::document::text::{Paragraph, normalize_paragraph, python_whitespace};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextLink {
    pub content_id: String,
    pub start: usize,
    pub end: usize,
    pub href: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentAnchor {
    pub content_id: String,
    pub fragment: String,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitationRange {
    pub start: usize,
    pub end: usize,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentSemantics {
    pub links: Vec<TextLink>,
    pub anchors: Vec<DocumentAnchor>,
    pub citations: Vec<CitationRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CharacterPosition {
    pub row: usize,
    pub col: usize,
}

struct LinkMark {
    start: CharacterPosition,
    end: Option<CharacterPosition>,
    href: String,
    citation: bool,
}

struct AnchorMark {
    position: CharacterPosition,
    fragment: String,
}

struct CitationContext {
    tag: String,
    depth: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct TextMark {
    pub start: CharacterPosition,
    pub end: Option<CharacterPosition>,
}

impl TextMark {
    pub fn is_valid(self) -> bool {
        self.end.is_some_and(|end| self.start <= end)
    }
}

#[derive(Default)]
pub(crate) struct LinkMetadata {
    links: Vec<LinkMark>,
    active_links: Vec<usize>,
    anchors: Vec<AnchorMark>,
    superscripts: Vec<TextMark>,
    citation_contexts: Vec<CitationContext>,
    tag_depths: BTreeMap<String, usize>,
}

pub(crate) fn attribute<'a>(tag: &'a HtmlTag, name: &str) -> Option<&'a str> {
    tag.attributes
        .iter()
        .rev()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.value.as_deref())
}

impl LinkMetadata {
    pub fn start(&mut self, tag: &HtmlTag, position: CharacterPosition) {
        let depth = self.tag_depths.entry(tag.name.clone()).or_default();
        *depth += 1;
        let semantics = attribute(tag, "epub:type").unwrap_or_default();
        let roles = attribute(tag, "role").unwrap_or_default();
        if semantics.split_whitespace().any(|value| value == "noteref")
            || roles.split_whitespace().any(|value| value == "doc-noteref")
            || attribute(tag, "type") == Some("note")
        {
            self.citation_contexts.push(CitationContext {
                tag: tag.name.clone(),
                depth: *depth,
            });
        }
        if let Some(fragment) = attribute(tag, "id")
            .filter(|value| !value.is_empty())
            .or_else(|| attribute(tag, "xml:id").filter(|value| !value.is_empty()))
            .or_else(|| attribute(tag, "name").filter(|value| !value.is_empty()))
        {
            self.anchors.push(AnchorMark {
                position,
                fragment: fragment.into(),
            });
        }
        if tag.name == "sup" {
            self.superscripts.push(TextMark {
                start: position,
                end: None,
            });
        }
        if tag.name == "a"
            && let Some(href) = attribute(tag, "href")
                .filter(|value| !value.is_empty())
                .or_else(|| attribute(tag, "xlink:href"))
        {
            self.active_links.push(self.links.len());
            self.links.push(LinkMark {
                start: position,
                end: None,
                href: href.into(),
                citation: !self.citation_contexts.is_empty(),
            });
        }
    }

    pub fn end(&mut self, tag: &str, position: CharacterPosition) {
        let depth = self.tag_depths.entry(tag.into()).or_default();
        self.citation_contexts
            .retain(|context| context.tag != tag || context.depth != *depth);
        *depth = depth.saturating_sub(1);
        if tag == "a"
            && let Some(index) = self.active_links.pop()
        {
            self.links[index].end = Some(position);
        }
        if tag == "sup"
            && let Some(mark) = self
                .superscripts
                .last_mut()
                .filter(|mark| mark.end.is_none())
        {
            mark.end = Some(position);
        }
    }

    pub fn finish(self, paragraphs: &[Paragraph], source: &SourceDocument) -> DocumentSemantics {
        let mut candidates = BTreeSet::new();
        let mut links = Vec::new();
        for mark in self.links {
            let Some(end) = mark.end else {
                continue;
            };
            let start = canonical_offset(mark.start, paragraphs);
            let end = canonical_offset(end, paragraphs);
            if mark.citation {
                candidates.insert((start, end));
            }
            if end > start {
                links.push(TextLink {
                    content_id: source.content_id.clone(),
                    start,
                    end,
                    href: mark.href,
                    label: character_slice(&source.text, start, end),
                });
            }
        }
        for mark in self.superscripts {
            let Some(end) = mark.end else {
                continue;
            };
            let start = canonical_offset(mark.start, paragraphs);
            let end = canonical_offset(end, paragraphs);
            if links
                .iter()
                .any(|link| link.start < end && link.end > start)
            {
                candidates.insert((start, end));
            }
        }
        let anchors = self
            .anchors
            .into_iter()
            .map(|mark| DocumentAnchor {
                content_id: source.content_id.clone(),
                fragment: mark.fragment,
                offset: canonical_offset(mark.position, paragraphs),
            })
            .collect();
        DocumentSemantics {
            citations: citation_ranges(candidates, &source.text),
            links,
            anchors,
        }
    }
}

fn citation_ranges(candidates: BTreeSet<(usize, usize)>, text: &str) -> Vec<CitationRange> {
    let mut result = Vec::<CitationRange>::new();
    for (start, end) in candidates {
        if result
            .iter()
            .any(|range| start < range.end && end > range.start)
        {
            continue;
        }
        let text = character_slice(text, start, end);
        let mut label = text.trim_matches(python_whitespace);
        if let Some(inner) = label
            .strip_prefix("^{")
            .and_then(|value| value.strip_suffix('}'))
        {
            label = inner;
        }
        label = label.trim_matches(['[', ']']);
        if !label.is_empty() && !label.contains('\n') {
            result.push(CitationRange {
                start,
                end,
                label: format!("[{label}]"),
            });
        }
    }
    result
}

pub(crate) fn canonical_offset(position: CharacterPosition, paragraphs: &[Paragraph]) -> usize {
    let paragraph = &paragraphs[position.row];
    paragraph.source_start
        + normalize_paragraph(
            &paragraph
                .raw_text
                .chars()
                .take(position.col)
                .collect::<String>(),
        )
        .chars()
        .count()
}

pub(crate) fn character_slice(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

pub(crate) struct SourceRange {
    pub start: usize,
    pub end: usize,
}

pub(crate) struct DisplayParagraph {
    pub text: String,
    pub ranges: Vec<SourceRange>,
}

impl DisplayParagraph {
    pub fn display_offset(&self, offset: usize) -> usize {
        self.ranges
            .iter()
            .position(|range| range.end > offset)
            .unwrap_or(self.ranges.len())
    }
}

pub(crate) fn project_paragraph(
    text: &str,
    offset: usize,
    citations: &[CitationRange],
) -> DisplayParagraph {
    let characters = text.chars().collect::<Vec<_>>();
    let mut displayed = String::new();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    for citation in citations {
        let Some(start) = citation.start.checked_sub(offset) else {
            continue;
        };
        let Some(end) = citation.end.checked_sub(offset) else {
            continue;
        };
        if start < cursor || end > characters.len() {
            continue;
        }
        displayed.extend(&characters[cursor..start]);
        ranges.extend((cursor..start).map(|index| SourceRange {
            start: offset + index,
            end: offset + index + 1,
        }));
        displayed.push_str(&citation.label);
        ranges.extend(citation.label.chars().map(|_| SourceRange {
            start: citation.start,
            end: citation.end,
        }));
        cursor = end;
    }
    displayed.extend(&characters[cursor..]);
    ranges.extend((cursor..characters.len()).map(|index| SourceRange {
        start: offset + index,
        end: offset + index + 1,
    }));
    DisplayParagraph {
        text: displayed,
        ranges,
    }
}
