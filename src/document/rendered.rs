use std::collections::BTreeMap;

use crate::document::layout::{LayoutOptions, TextLayout, layout_document};
use crate::document::text::{ParsedDocument, python_whitespace};

#[derive(Clone, Debug)]
struct RowFormatting {
    spans: Vec<crate::document::layout::StyleSpan>,
}

#[derive(Clone, Debug)]
pub struct RenderedDocument {
    pub index: usize,
    pub width: usize,
    pub parsed: ParsedDocument,
    pub layout: TextLayout,
    letter_prefix: Vec<usize>,
    formatting_rows: BTreeMap<usize, RowFormatting>,
}

impl RenderedDocument {
    pub fn new(index: usize, parsed: ParsedDocument, width: usize) -> Self {
        Self::with_layout(
            index,
            parsed,
            LayoutOptions {
                width,
                starting_row: 0,
                image_rows: 1,
            },
        )
    }

    pub fn with_layout(index: usize, parsed: ParsedDocument, options: LayoutOptions) -> Self {
        let width = options.width;
        let layout = layout_document(&parsed, options);
        let mut letter_prefix = vec![0];
        let mut count = 0;
        for character in parsed.source.text.chars() {
            count += usize::from(!python_whitespace(character));
            letter_prefix.push(count);
        }
        let mut formatting_rows = BTreeMap::<usize, RowFormatting>::new();
        for span in &layout.formatting {
            formatting_rows
                .entry(span.row)
                .or_insert_with(|| RowFormatting { spans: Vec::new() })
                .spans
                .push(span.clone());
        }
        Self {
            index,
            parsed,
            layout,
            width,
            letter_prefix,
            formatting_rows,
        }
    }

    pub fn styles_on_row(&self, row: usize) -> &[crate::document::layout::StyleSpan] {
        self.formatting_rows
            .get(&row)
            .map_or(&[], |row| row.spans.as_slice())
    }

    pub fn letters_before(&self, offset: usize) -> usize {
        self.letter_prefix
            .get(offset)
            .copied()
            .unwrap_or_else(|| self.letter_prefix.last().copied().unwrap_or(0))
    }
}
