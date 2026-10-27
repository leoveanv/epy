use std::collections::VecDeque;
use std::sync::Arc;

use crate::Result;
use crate::document::RenderedDocument;
use crate::document::layout::LayoutOptions;
use crate::document::text::ParsedDocument;

pub(super) struct DocumentCache {
    entries: VecDeque<Arc<RenderedDocument>>,
    entries_max: usize,
    bytes_max: usize,
}

impl DocumentCache {
    pub(super) fn new(entries_max: usize, bytes_max: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            entries_max,
            bytes_max,
        }
    }

    pub(super) fn prepare(
        &mut self,
        index: usize,
        options: LayoutOptions,
        load: impl FnOnce() -> Result<ParsedDocument>,
    ) -> Result<Arc<RenderedDocument>> {
        if let Some(document) = self.get(index, options) {
            return Ok(document);
        }
        let parsed = match self.parsed(index) {
            Some(parsed) => parsed.clone(),
            None => load()?,
        };
        let document = Arc::new(RenderedDocument::with_layout(index, parsed, options));
        self.insert(document.clone());
        Ok(document)
    }

    fn get(&mut self, index: usize, options: LayoutOptions) -> Option<Arc<RenderedDocument>> {
        let position = self.entries.iter().position(|document| {
            document.index == index
                && document.width == options.width
                && document.layout.text_lines.image_layout().height() == options.image_rows
        })?;
        let document = self.entries.remove(position)?;
        self.entries.push_back(document.clone());
        Some(document)
    }

    pub(super) fn parsed(&self, index: usize) -> Option<&ParsedDocument> {
        self.entries
            .iter()
            .find(|document| document.index == index)
            .map(|document| &document.parsed)
    }

    fn insert(&mut self, document: Arc<RenderedDocument>) {
        self.entries.push_back(document);
        while self.entries.len() > self.entries_max || cache_bytes(&self.entries) > self.bytes_max {
            self.entries.pop_front();
        }
    }
}

fn cache_bytes(cache: &VecDeque<Arc<RenderedDocument>>) -> usize {
    cache
        .iter()
        .map(|document| {
            document.parsed.source.text.len()
                + document
                    .parsed
                    .references
                    .headings
                    .iter()
                    .map(|heading| std::mem::size_of_val(heading) + heading.title.len())
                    .sum::<usize>()
                + document
                    .parsed
                    .references
                    .pages
                    .iter()
                    .map(|page| {
                        std::mem::size_of_val(page) + page.label.as_ref().map_or(0, String::len)
                    })
                    .sum::<usize>()
                + document.layout.text_lines.storage_bytes()
                + document
                    .layout
                    .source_cells
                    .iter()
                    .map(|cell| {
                        std::mem::size_of_val(cell) + cell.text.len() + cell.content_id.len()
                    })
                    .sum::<usize>()
        })
        .sum()
}
