use crate::annotations::{AnnotationReference, TextAnchor};
use crate::book::BookNavigation;
use crate::document::text::ParsedDocument;
use crate::document::{ReferenceHeading, ReferencePage};
use crate::{Error, Result};

pub struct ReferenceRequest<'a> {
    pub anchor: &'a TextAnchor,
    pub document: &'a ParsedDocument,
    pub navigation: &'a BookNavigation,
    pub content_order: usize,
    pub fallback_headings: &'a [String],
}

fn target_offset(document: &ParsedDocument, fragment: Option<&str>) -> Option<usize> {
    match fragment {
        None => Some(0),
        Some(fragment) => document
            .semantics
            .anchors
            .iter()
            .find(|anchor| anchor.fragment == fragment)
            .map(|anchor| anchor.offset),
    }
}

fn reference_headings(request: &ReferenceRequest<'_>) -> Vec<String> {
    let mut boundary = 0;
    let mut chosen = None;
    for target in &request.navigation.sections {
        if target.content_id != request.document.source.content_id {
            continue;
        }
        let Some(offset) = target_offset(request.document, target.fragment.as_deref())
            .filter(|offset| *offset <= request.anchor.start())
        else {
            continue;
        };
        if chosen.is_none() || offset >= boundary {
            boundary = offset;
            chosen = Some(target);
        }
    }
    let mut stack = chosen.map_or_else(Vec::new, |target| {
        target
            .headings
            .iter()
            .enumerate()
            .map(|(index, title)| ReferenceHeading {
                offset: boundary,
                title: title.clone(),
                level: index + 1,
            })
            .collect()
    });
    for heading in &request.document.references.headings {
        if heading.offset < boundary || heading.offset > request.anchor.start() {
            continue;
        }
        if let Some(last) = stack.last_mut().filter(|last| last.title == heading.title) {
            *last = heading.clone();
            continue;
        }
        while stack.last().is_some_and(|last| last.level >= heading.level) {
            stack.pop();
        }
        stack.push(heading.clone());
    }
    if stack.is_empty() {
        if request.fallback_headings.is_empty() {
            vec![format!("Chapter {}", request.content_order + 1)]
        } else {
            request.fallback_headings.to_vec()
        }
    } else {
        stack.into_iter().map(|heading| heading.title).collect()
    }
}

fn reference_pages(request: &ReferenceRequest<'_>) -> Vec<ReferencePage> {
    let mut pages = request
        .document
        .references
        .pages
        .iter()
        .map(|page| (page.offset, page.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for target in &request.navigation.pages {
        if target.content_id != request.document.source.content_id {
            continue;
        }
        if let Some(offset) = target_offset(request.document, target.fragment.as_deref())
            && pages.get(&offset).is_none_or(|page| page.label.is_none())
        {
            pages.insert(
                offset,
                ReferencePage {
                    offset,
                    label: Some(target.label.clone()),
                },
            );
        }
    }
    pages.into_values().collect()
}

fn page_at(pages: &[ReferencePage], offset: usize) -> Option<String> {
    pages
        .iter()
        .rev()
        .find(|page| page.offset <= offset)
        .and_then(|page| page.label.clone())
}

pub fn resolve_reference(request: ReferenceRequest<'_>) -> Result<AnnotationReference> {
    if request.anchor.resolve(&request.document.source).is_none() {
        return Err(Error::Compatibility(
            "The saved quotation does not match this document.".into(),
        ));
    }
    let pages = reference_pages(&request);
    let first = page_at(&pages, request.anchor.start());
    let last = page_at(&pages, request.anchor.end() - 1);
    Ok(AnnotationReference {
        headings: reference_headings(&request),
        location: request.anchor.stable_location(request.content_order),
        page_end: if first.is_some() && first != last {
            last
        } else {
            None
        },
        page_start: first,
        version: 1,
    })
}
