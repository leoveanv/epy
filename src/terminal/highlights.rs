use std::collections::BTreeMap;
use std::ops::Range;

use crate::annotations::Annotation;
use crate::reading::VisibleRow;

pub(super) struct Highlights<'a> {
    documents: BTreeMap<&'a str, Vec<Range<usize>>>,
}

impl<'a> Highlights<'a> {
    pub(super) fn visible(rows: &[VisibleRow<'_>], annotations: &'a [Annotation]) -> Self {
        if annotations.is_empty() {
            return Self {
                documents: BTreeMap::new(),
            };
        }
        let bounds = visible_bounds(rows);
        let mut documents = BTreeMap::<&str, Vec<Range<usize>>>::new();
        for annotation in annotations {
            let anchor = &annotation.anchor;
            let Some(visible) = bounds.get(anchor.content_id()) else {
                continue;
            };
            if anchor.start() < visible.end && anchor.end() > visible.start {
                documents
                    .entry(anchor.content_id())
                    .or_default()
                    .push(anchor.start()..anchor.end());
            }
        }
        for ranges in documents.values_mut() {
            *ranges = merge_ranges(std::mem::take(ranges));
        }
        Self { documents }
    }

    pub(super) fn overlaps(&self, content_id: &str, source: Range<usize>) -> bool {
        let Some(ranges) = self.documents.get(content_id) else {
            return false;
        };
        let index = ranges.partition_point(|range| range.start < source.end);
        index > 0 && ranges[index - 1].end > source.start
    }
}

fn visible_bounds<'a>(rows: &'a [VisibleRow<'_>]) -> BTreeMap<&'a str, Range<usize>> {
    let mut bounds = BTreeMap::<&str, Range<usize>>::new();
    for row in rows {
        let cells = row.document.layout.cells_on_row(row.row);
        let (Some(first), Some(last)) = (cells.first(), cells.last()) else {
            continue;
        };
        bounds
            .entry(&row.document.parsed.source.content_id)
            .and_modify(|range| {
                range.start = range.start.min(first.start);
                range.end = range.end.max(last.end);
            })
            .or_insert(first.start..last.end);
    }
    bounds
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged = Vec::<Range<usize>>::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}
