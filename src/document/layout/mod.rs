mod rows;
use rows::ImageSource;
pub use rows::{ImageLayout, ImageSlotPosition, TextLines};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::document::links::{DisplayParagraph, project_paragraph};
use crate::document::text::{
    Paragraph, ParagraphRole, ParsedDocument, cell_width, graphemes, python_whitespace, wrap_text,
};

#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions {
    pub width: usize,
    pub starting_row: usize,
    pub image_rows: usize,
}

impl LayoutOptions {
    pub fn reader(width: usize, inline_images: bool) -> Self {
        // #NOTE: Match the reference reader's width-based inline slots without making image decoding affect reflow.
        let image_rows = if inline_images {
            (width / 4).clamp(3, 24)
        } else {
            1
        };
        Self {
            width,
            starting_row: 0,
            image_rows,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontStyle {
    Normal,
    Bold,
    Italic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCell {
    pub content_id: String,
    pub start: usize,
    pub end: usize,
    pub row: usize,
    pub col: usize,
    pub width: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleSpan {
    pub row: usize,
    pub col: usize,
    pub n_letters: usize,
    pub style: FontStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParagraphStyle {
    pub row: usize,
    pub col: usize,
    pub n_letters: usize,
    pub style: FontStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLayout {
    pub content_id: String,
    pub text_lines: TextLines,
    pub source_cells: Vec<SourceCell>,
    pub formatting: Vec<StyleSpan>,
    pub image_maps: BTreeMap<usize, String>,
}

impl TextLayout {
    pub fn cells_on_row(&self, row: usize) -> &[SourceCell] {
        let start = self.source_cells.partition_point(|cell| cell.row < row);
        let end = self.source_cells.partition_point(|cell| cell.row <= row);
        &self.source_cells[start..end]
    }

    pub fn row_for_offset(&self, offset: usize) -> usize {
        let index = self.source_cells.partition_point(|cell| cell.end <= offset);
        let text = self.source_cells.get(index);
        self.text_lines
            .image_layout()
            .source_row_before(offset, text.map(|cell| cell.start))
            .unwrap_or_else(|| {
                text.or_else(|| self.source_cells.last())
                    .map_or(0, |cell| cell.row)
            })
    }

    pub fn first_cell_from_row(&self, row: usize) -> Option<&SourceCell> {
        self.source_cells
            .get(self.source_cells.partition_point(|cell| cell.row < row))
    }
}

fn center(value: &str, width: usize) -> String {
    let margin = width.saturating_sub(value.chars().count());
    let left = margin / 2 + (margin & width & 1);
    format!("{}{value}{}", " ".repeat(left), " ".repeat(margin - left))
}

fn split_preformatted(value: &str) -> Vec<&str> {
    let mut lines = value
        .split([
            '\n', '\u{001c}', '\u{001d}', '\u{001e}', '\u{0085}', '\u{2028}', '\u{2029}',
        ])
        .collect::<Vec<_>>();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

fn paragraph_lines(paragraph: &Paragraph, text: &str, width: usize) -> Vec<String> {
    let mut lines = match paragraph.role {
        ParagraphRole::Heading => vec![format!(
            "{}{text}",
            " ".repeat(width.saturating_sub(cell_width(text)) / 2)
        )],
        ParagraphRole::Indented => wrap_text(text, width.saturating_sub(3))
            .into_iter()
            .map(|line| format!("   {line}"))
            .collect(),
        ParagraphRole::Bullet => {
            let wrapped = wrap_text(text, width.saturating_sub(3));
            wrapped
                .iter()
                .map(|line| {
                    format!(
                        "{}{line}",
                        if Some(line) == wrapped.first() {
                            " - "
                        } else {
                            "   "
                        }
                    )
                })
                .collect()
        }
        ParagraphRole::Preformatted => split_preformatted(text)
            .into_iter()
            .flat_map(|line| wrap_text(line, width.saturating_sub(6)))
            .map(|line| format!("   {line}"))
            .collect(),
        ParagraphRole::Image => vec![center(text, width)],
        ParagraphRole::Body => wrap_text(text, width),
    };
    lines.push(String::new());
    lines
}

struct CellMapping<'a> {
    content_id: &'a str,
    projection: &'a DisplayParagraph,
    rendered: &'a [String],
    starting_row: usize,
    indentation: usize,
}

fn map_cells(mapping: CellMapping<'_>) -> Vec<SourceCell> {
    let mut cells = Vec::new();
    let mut cursor = 0;
    for (row_index, line) in mapping.rendered.iter().enumerate() {
        let fragment = line
            .get(mapping.indentation..)
            .unwrap_or_default()
            .trim_matches(python_whitespace);
        if fragment.is_empty() {
            continue;
        }
        let Some(location) = mapping.projection.text[cursor..]
            .find(fragment)
            .map(|index| cursor + index)
        else {
            continue;
        };
        let fragment_col = line.find(fragment).unwrap_or_default();
        let mut col = cell_width(&line[..fragment_col]);
        let mut position = mapping.projection.text[..location].chars().count();
        for cluster in graphemes(fragment) {
            let count = cluster.chars().count();
            let width = cell_width(cluster);
            let first = &mapping.projection.ranges[position];
            let last = &mapping.projection.ranges[position + count - 1];
            cells.push(SourceCell {
                content_id: mapping.content_id.into(),
                start: first.start,
                end: last.end,
                row: mapping.starting_row + row_index,
                col,
                width,
                text: cluster.into(),
            });
            position += count;
            col += width;
        }
        cursor = location + fragment.len();
    }
    cells
}

fn wrapped_styles(
    lines: &[String],
    span: ParagraphStyle,
    starting_row: usize,
    left: usize,
) -> Vec<StyleSpan> {
    let start = span.col;
    let end = start + span.n_letters;
    let mut previous = 0;
    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let length = line.chars().count() + 1;
        let current = previous + length;
        let contains_start = (previous..current).contains(&start);
        let contains_end = (previous..current).contains(&end);
        let values = if contains_start && contains_end {
            Some((start - previous + left, span.n_letters))
        } else if contains_start {
            Some((start - previous + left, current - start - 1))
        } else if contains_end {
            Some((left, end - previous + 1))
        } else if (start..end).contains(&previous) && (start..end).contains(&current) {
            Some((left, length - 1))
        } else {
            None
        };
        if let Some((col, n_letters)) = values {
            result.push(StyleSpan {
                row: starting_row + index,
                col,
                n_letters,
                style: span.style,
            });
        }
        if previous > end {
            break;
        }
        previous = current;
    }
    result
}

struct LayoutDraft {
    content_id: String,
    text_lines: Vec<String>,
    source_cells: Vec<SourceCell>,
    formatting: Vec<StyleSpan>,
    image_maps: BTreeMap<usize, String>,
    image_sources: Vec<ImageSource>,
}

impl LayoutDraft {
    fn finish(mut self, options: LayoutOptions) -> TextLayout {
        let images = ImageLayout::new(self.image_sources, options.image_rows);
        for cell in &mut self.source_cells {
            cell.row = images.display_row(cell.row);
        }
        for span in &mut self.formatting {
            span.row = images.display_row(span.row);
        }
        let image_maps = self
            .image_maps
            .into_iter()
            .map(|(row, path)| (images.display_row(row), path))
            .collect();
        TextLayout {
            content_id: self.content_id,
            text_lines: TextLines::new(self.text_lines, images, options.starting_row),
            source_cells: self.source_cells,
            formatting: self.formatting,
            image_maps,
        }
    }
}

pub fn layout_document(document: &ParsedDocument, options: LayoutOptions) -> TextLayout {
    let mut layout = LayoutDraft {
        content_id: document.source.content_id.clone(),
        text_lines: Vec::new(),
        source_cells: Vec::new(),
        formatting: Vec::new(),
        image_maps: BTreeMap::new(),
        image_sources: Vec::new(),
    };
    for (index, paragraph) in document.paragraphs.iter().enumerate() {
        if document.suppressed_images.binary_search(&index).is_ok() {
            continue;
        }
        let projection = project_paragraph(
            &paragraph.text,
            paragraph.source_start,
            &document.semantics.citations,
        );
        let lines = paragraph_lines(paragraph, &projection.text, options.width);
        let first_row = options.starting_row + layout.text_lines.len();
        if paragraph.role == ParagraphRole::Heading {
            layout
                .formatting
                .extend(lines.iter().enumerate().map(|(row, line)| StyleSpan {
                    row: first_row + row,
                    col: 0,
                    n_letters: line.chars().count(),
                    style: FontStyle::Bold,
                }));
        } else if paragraph.role == ParagraphRole::Image {
            if let Some(path) = &paragraph.image_path {
                layout.image_maps.insert(first_row, path.clone());
                layout.image_sources.push(ImageSource {
                    start: paragraph.source_start,
                    end: paragraph.source_start + paragraph.text.chars().count(),
                    original_row: first_row,
                });
            }
            layout.formatting.push(StyleSpan {
                row: first_row,
                col: 0,
                n_letters: lines[0].chars().count(),
                style: FontStyle::Bold,
            });
        }
        let indentation = if matches!(
            paragraph.role,
            ParagraphRole::Bullet | ParagraphRole::Indented | ParagraphRole::Preformatted
        ) {
            3
        } else {
            0
        };
        if paragraph.role != ParagraphRole::Image {
            layout.source_cells.extend(map_cells(CellMapping {
                content_id: &document.source.content_id,
                projection: &projection,
                rendered: &lines,
                starting_row: first_row,
                indentation,
            }));
        }
        let left = if matches!(
            paragraph.role,
            ParagraphRole::Bullet | ParagraphRole::Indented
        ) {
            3
        } else {
            0
        };
        for span in document.styles.iter().filter(|span| span.row == index) {
            let start = projection.display_offset(paragraph.source_start + span.col);
            let end = projection.display_offset(paragraph.source_start + span.col + span.n_letters);
            layout.formatting.extend(wrapped_styles(
                &lines,
                ParagraphStyle {
                    row: index,
                    col: start,
                    n_letters: end.saturating_sub(start),
                    style: span.style,
                },
                first_row,
                left,
            ));
        }
        layout.text_lines.extend(lines);
    }
    layout.text_lines.push(center("***", options.width));
    layout.finish(options)
}
