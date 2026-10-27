use super::{Paragraph, RawParagraph, python_whitespace};
use crate::document::html::HtmlTag;
use crate::document::links::{CharacterPosition, attribute, canonical_offset};
use crate::document::{DocumentReferences, ReferenceHeading, ReferencePage, clean_page_label};
use crate::{Error, Result};
use std::collections::BTreeMap;

struct HeadingMark {
    start: CharacterPosition,
    level: usize,
    title: String,
}
struct PageMark {
    start: CharacterPosition,
    label: Option<String>,
}
struct OpenPage {
    index: usize,
    tag: String,
    depth: usize,
}

#[derive(Default)]
pub(super) struct ReferenceMetadata {
    headings: Vec<HeadingMark>,
    pages: Vec<PageMark>,
    open_pages: Vec<OpenPage>,
    depths: BTreeMap<String, usize>,
    active_heading: Option<HeadingMark>,
}

fn text_since(paragraphs: &[RawParagraph], start: CharacterPosition) -> String {
    let text = paragraphs
        .iter()
        .enumerate()
        .skip(start.row)
        .map(|(row, paragraph)| {
            if row == start.row {
                paragraph.text.chars().skip(start.col).collect::<String>()
            } else {
                paragraph.text.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    text.split(python_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl ReferenceMetadata {
    pub fn start(&mut self, tag: &HtmlTag, position: CharacterPosition) -> Result<()> {
        if !tag.self_closing {
            *self.depths.entry(tag.name.clone()).or_default() += 1;
        }
        if tag.name.len() == 2
            && tag.name.as_bytes()[0] == b'h'
            && matches!(tag.name.as_bytes()[1], b'1'..=b'6')
        {
            self.active_heading = Some(HeadingMark {
                start: position,
                level: (tag.name.as_bytes()[1] - b'0') as usize,
                title: String::new(),
            });
        }
        let pagebreak = tag
            .attributes
            .iter()
            .filter(|attribute| attribute.name.ends_with(":type"))
            .filter_map(|attribute| attribute.value.as_deref())
            .flat_map(str::split_whitespace)
            .any(|kind| kind == "pagebreak")
            || attribute(tag, "role")
                .is_some_and(|roles| roles.split_whitespace().any(|role| role == "doc-pagebreak"));
        if pagebreak {
            if self.pages.len() >= 100_000 || self.open_pages.len() >= 64 {
                return Err(Error::Compatibility(
                    "The document exceeds its reference metadata limits.".into(),
                ));
            }
            let label = attribute(tag, "title")
                .and_then(clean_page_label)
                .or_else(|| attribute(tag, "aria-label").and_then(clean_page_label));
            if label.is_none() && !tag.self_closing {
                self.open_pages.push(OpenPage {
                    index: self.pages.len(),
                    tag: tag.name.clone(),
                    depth: self.depths.get(&tag.name).copied().unwrap_or(0),
                });
            }
            self.pages.push(PageMark {
                start: position,
                label,
            });
        }
        Ok(())
    }

    pub fn end(&mut self, tag: &str, paragraphs: &[RawParagraph]) {
        if self
            .active_heading
            .as_ref()
            .is_some_and(|heading| tag == format!("h{}", heading.level))
            && let Some(mut heading) = self.active_heading.take()
        {
            heading.title = text_since(paragraphs, heading.start);
            self.headings.push(heading);
        }
        let depth = self.depths.get(tag).copied().unwrap_or(0);
        for page in &self.open_pages {
            if page.tag == tag && page.depth == depth {
                let marker = &mut self.pages[page.index];
                marker.label = clean_page_label(&text_since(paragraphs, marker.start));
            }
        }
        self.open_pages
            .retain(|page| page.tag != tag || page.depth != depth);
        if let Some(depth) = self.depths.get_mut(tag) {
            *depth = depth.saturating_sub(1);
        }
    }

    pub fn finish(self, paragraphs: &[Paragraph]) -> DocumentReferences {
        DocumentReferences {
            headings: self
                .headings
                .into_iter()
                .filter(|heading| !heading.title.is_empty())
                .map(|heading| ReferenceHeading {
                    offset: canonical_offset(heading.start, paragraphs),
                    title: heading.title,
                    level: heading.level,
                })
                .collect(),
            pages: self
                .pages
                .into_iter()
                .map(|page| ReferencePage {
                    offset: canonical_offset(page.start, paragraphs),
                    label: page.label,
                })
                .collect(),
        }
    }
}
