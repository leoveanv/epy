use crate::document::text::wrap_text;

#[derive(Clone, Copy, Debug)]
pub struct ReadingLayout {
    pub reading_x: usize,
    pub notes_x: Option<usize>,
    pub notes_width: usize,
}

pub fn reading_layout(columns: usize, textwidth: usize, notes_width: usize) -> ReadingLayout {
    let centered = columns.saturating_sub(textwidth) / 2;
    if columns < textwidth + notes_width + 10 {
        return ReadingLayout {
            reading_x: centered,
            notes_x: None,
            notes_width,
        };
    }
    let reading_x = centered.min(columns.saturating_sub(textwidth + notes_width + 8));
    ReadingLayout {
        reading_x,
        notes_x: Some(reading_x + textwidth + 6),
        notes_width,
    }
}

#[derive(Clone, Debug)]
pub struct MarginInput {
    pub annotation_id: String,
    pub source_row: usize,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct PlacedNote {
    pub annotation_id: String,
    pub source_row: usize,
    pub row: usize,
    pub lines: Vec<String>,
}

pub fn place_notes(notes: &[MarginInput], width: usize) -> Vec<PlacedNote> {
    let mut bottom = 0;
    notes
        .iter()
        .map(|note| {
            let lines = note
                .text
                .split('\n')
                .flat_map(|paragraph| {
                    let lines = wrap_text(paragraph, width);
                    if lines.is_empty() {
                        vec![String::new()]
                    } else {
                        lines
                    }
                })
                .collect::<Vec<_>>();
            let row = bottom.max(note.source_row);
            bottom = row + lines.len() + 1;
            PlacedNote {
                annotation_id: note.annotation_id.clone(),
                source_row: note.source_row,
                row,
                lines,
            }
        })
        .collect()
}
