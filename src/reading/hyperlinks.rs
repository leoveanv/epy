use std::collections::{BTreeMap, BTreeSet};
use url::Url;

use crate::document::links::TextLink;
use crate::reading::{DocumentRow, ReadingWindow};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct LinkHint {
    pub number: usize,
    pub link: TextLink,
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug)]
pub struct HintPlacement {
    pub number: usize,
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug)]
pub struct HyperlinkRequest {
    pub link: TextLink,
    pub width: usize,
    pub continuous_rows: Option<usize>,
}

pub fn link_hints(window: &ReadingWindow, origin: DocumentRow, rows: usize) -> Vec<LinkHint> {
    let mut seen = BTreeSet::new();
    let mut hints = Vec::new();
    for row in 0..rows {
        let Some(visible) = window.row(origin, row) else {
            break;
        };
        for (index, link) in visible.document.parsed.semantics.links.iter().enumerate() {
            if seen.contains(&(visible.document.index, index)) {
                continue;
            }
            if let Some(cell) = visible
                .document
                .layout
                .cells_on_row(visible.row)
                .iter()
                .find(|cell| cell.start < link.end && cell.end > link.start)
            {
                hints.push(LinkHint {
                    number: hints.len() + 1,
                    link: link.clone(),
                    row,
                    column: cell.col,
                });
                seen.insert((visible.document.index, index));
            }
        }
    }
    hints
}

pub fn place_hints(hints: &[LinkHint], width: usize) -> Option<Vec<HintPlacement>> {
    let mut occupied = BTreeMap::<usize, BTreeSet<usize>>::new();
    let mut placements = Vec::with_capacity(hints.len());
    for hint in hints {
        let label_width = hint.number.to_string().len() + 2;
        let last = width.checked_sub(label_width)?;
        let preferred = hint.column.min(last);
        let used = occupied.entry(hint.row).or_default();
        let column = (preferred..=last)
            .chain(0..preferred)
            .find(|column| (*column..*column + label_width).all(|cell| !used.contains(&cell)))?;
        used.extend(column..column + label_width);
        placements.push(HintPlacement {
            number: hint.number,
            row: hint.row,
            column,
        });
    }
    Some(placements)
}

pub fn external_url(from: &str, href: &str) -> Result<Option<Url>> {
    if href.chars().any(|character| character.is_ascii_control()) {
        return Err(Error::Compatibility(
            "The link contains control characters.".into(),
        ));
    }
    if href.starts_with('#') {
        return Ok(None);
    }
    let target = if let Ok(base) = Url::parse(from)
        && matches!(base.scheme(), "http" | "https")
    {
        Some(
            base.join(href)
                .map_err(|error| Error::Compatibility(format!("Invalid link URL: {error}")))?,
        )
    } else if href.starts_with("//") {
        Some(
            Url::parse(&format!("https:{href}"))
                .map_err(|error| Error::Compatibility(format!("Invalid link URL: {error}")))?,
        )
    } else {
        match Url::parse(href) {
            Ok(url) => Some(url),
            Err(url::ParseError::RelativeUrlWithoutBase) => None,
            Err(error) => return Err(Error::Compatibility(format!("Invalid link URL: {error}"))),
        }
    };
    if let Some(url) = &target
        && (!matches!(url.scheme(), "http" | "https") || url.host_str().is_none())
    {
        return Err(Error::Compatibility(
            "Only HTTP and HTTPS links can open in a browser.".into(),
        ));
    }
    Ok(target)
}
