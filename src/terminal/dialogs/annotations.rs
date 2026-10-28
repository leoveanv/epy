use std::collections::BTreeMap;
use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::annotations::{MarginInput, PlacedNote, ReadingLayout, note_view_lines, place_notes};
use crate::app::modes::annotations::{AnnotationMode, NoteEditor};
use crate::app::{ReaderMode, ReaderModel};
use crate::document::text::clip_text;

use super::render::safe_text;
use crate::terminal::widgets::editor::{EditorRows, TextClipping, draw_rows};
use crate::terminal::widgets::list::{ListPaint, draw_list, draw_scrolled_list, popup};

pub(in crate::terminal) fn status_hint(model: &ReaderModel) -> Option<String> {
    let ReaderMode::Annotations(mode) = model.view().mode else {
        return None;
    };
    Some(match mode {
        AnnotationMode::Selecting(_) => "Select: hjkl/arrows  w/b words  Ctrl-U/D half page  Space start  Enter save  n note  d delete  v/Esc exit".into(),
        AnnotationMode::Editing(_) => "Note | Enter newline  Tab controls  Esc cancel".into(),
        AnnotationMode::QuitNote(_) => "Discard unsaved note and quit? y confirms; any other key keeps the draft".into(),
        AnnotationMode::List { .. } => "Annotations | Enter jump  e edit  d delete  f read note  r copy  Esc close".into(),
        AnnotationMode::Choosing(_) => "Overlapping highlights | j/k choose  Enter select  Esc cancel".into(),
        AnnotationMode::Viewing(_) => "Quotation and note | j/k scroll  Esc return".into(),
        AnnotationMode::Confirming(confirmation) => format!("{}  y confirms; any other key cancels", confirmation.message),
        AnnotationMode::Pending(pending) => pending.error.as_ref().map_or_else(|| "Saving annotation…".into(), |error| format!("Save failed: {error}. Enter retries; Esc returns to draft")),
    })
}

pub(in crate::terminal) fn draw_notes(
    frame: &mut Frame<'_>,
    model: &ReaderModel,
    geometry: ReadingLayout,
    style: Style,
) {
    let Some(notes_x) = geometry.notes_x else {
        return;
    };
    let draft = model.note_editor();
    let mut inputs = Vec::new();
    let mut note_rows = NoteRows::new(model);
    for annotation in &model.view().info.annotations {
        let text = if let Some(editor) =
            draft.filter(|editor| editor.target.id() == Some(annotation.id.as_str()))
        {
            editor.state.text()
        } else {
            &annotation.note
        };
        if text.is_empty() {
            continue;
        }
        if let Some(row) = note_rows.row(model, &annotation.anchor) {
            inputs.push(MarginInput {
                annotation_id: annotation.id.clone(),
                source_row: row,
                text: text.into(),
            });
        }
    }
    if let Some(editor) =
        draft.filter(|editor| editor.target.id().is_none() && !editor.state.text().is_empty())
        && let Some(row) = note_rows.row(model, editor.target.anchor())
    {
        inputs.push(MarginInput {
            annotation_id: "draft".into(),
            source_row: row,
            text: editor.state.text().into(),
        });
    }
    inputs.sort_by_key(|note| note.source_row);
    let height = model
        .reading_height()
        .min(frame.area().height.saturating_sub(1) as usize);
    let visible_count = inputs.len().min(max_visible_notes(height));
    let mut overflow = visible_count < inputs.len();
    let notes = place_notes(&inputs[..visible_count], geometry.notes_width);
    draw_note_connectors(
        frame,
        &notes,
        ConnectorGutter {
            start_x: geometry.reading_x + model.view().width + 1,
            end_x: notes_x.saturating_sub(2),
            height,
            ascii: model
                .view()
                .configuration
                .settings
                .annotation_ascii_connectors,
            style: style.add_modifier(Modifier::DIM),
        },
    );
    for note in notes {
        let focused = model.focused_annotation() == Some(note.annotation_id.as_str())
            || draft.is_some_and(|editor| editor.target.id() == Some(&note.annotation_id))
            || note.annotation_id == "draft";
        let note_style = if focused {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        };
        for (index, line) in note.lines.iter().enumerate() {
            let row = note.row + index;
            if row >= height {
                overflow = true;
                break;
            }
            frame.render_widget(
                Paragraph::new(clip_text(&safe_text(line), geometry.notes_width)).style(note_style),
                Rect::new(notes_x as u16, row as u16, geometry.notes_width as u16, 1),
            );
        }
    }
    if overflow && height > 0 {
        frame.render_widget(
            Paragraph::new(clip_text("More notes below; a: list", geometry.notes_width))
                .style(style.add_modifier(Modifier::BOLD)),
            Rect::new(
                notes_x as u16,
                height.saturating_sub(1) as u16,
                geometry.notes_width as u16,
                1,
            ),
        );
    }
}

#[derive(Clone, Copy)]
struct ConnectorGutter {
    start_x: usize,
    end_x: usize,
    height: usize,
    ascii: bool,
    style: Style,
}

struct ConnectorLane {
    column: usize,
    bottom: Option<usize>,
}

impl ConnectorLane {
    fn available(&self, source_row: usize) -> bool {
        self.bottom.is_none_or(|bottom| bottom < source_row)
    }
}

fn draw_note_connectors(frame: &mut Frame<'_>, notes: &[PlacedNote], gutter: ConnectorGutter) {
    let mut lanes = (gutter.start_x..=gutter.end_x)
        .rev()
        .step_by(2)
        .chain((gutter.start_x..gutter.end_x).rev().step_by(2))
        .map(|column| ConnectorLane {
            column,
            bottom: None,
        })
        .collect::<Vec<_>>();
    for note in notes {
        let Some(lane) = lanes
            .iter()
            .position(|lane| lane.available(note.source_row))
            .or_else(|| {
                lanes
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, lane)| lane.bottom)
                    .map(|(index, _)| index)
            })
        else {
            return;
        };
        let corner_x = lanes[lane].column;
        if note.row > note.source_row {
            lanes[lane].bottom = Some(note.row);
        }
        draw_note_connector(frame, note, gutter, corner_x);
    }
}

fn draw_note_connector(
    frame: &mut Frame<'_>,
    note: &PlacedNote,
    gutter: ConnectorGutter,
    corner_x: usize,
) {
    if note.source_row >= gutter.height {
        return;
    }
    draw_connector_dots(
        frame,
        gutter,
        note.source_row,
        gutter.start_x..gutter.end_x + 1,
    );
    if note.row <= note.source_row {
        return;
    }
    draw_connector_cell(frame, gutter, corner_x, note.source_row, "╮");
    for row in note.source_row + 1..note.row.min(gutter.height) {
        draw_connector_cell(frame, gutter, corner_x, row, "┊");
    }
    if note.row < gutter.height {
        draw_connector_cell(frame, gutter, corner_x, note.row, "╰");
        draw_connector_dots(frame, gutter, note.row, corner_x + 1..gutter.end_x + 1);
    }
}

fn draw_connector_dots(
    frame: &mut Frame<'_>,
    gutter: ConnectorGutter,
    row: usize,
    columns: Range<usize>,
) {
    for column in columns {
        let Some(cell) = frame.buffer_mut().cell_mut((column as u16, row as u16)) else {
            continue;
        };
        if matches!(cell.symbol(), " " | ".") {
            cell.set_symbol(".").set_style(gutter.style);
        }
    }
}

fn draw_connector_cell(
    frame: &mut Frame<'_>,
    gutter: ConnectorGutter,
    column: usize,
    row: usize,
    symbol: &str,
) {
    let Some(cell) = frame.buffer_mut().cell_mut((column as u16, row as u16)) else {
        return;
    };
    // #NOTE: When all gutter lanes are occupied, explicit junctions preserve each note's branch.
    let symbol = match (cell.symbol(), symbol) {
        ("╰" | "+", "┊") | ("┊" | ":", "╰") | ("├", _) => "├",
        _ => symbol,
    };
    let symbol = if gutter.ascii {
        if symbol == "┊" { ":" } else { "+" }
    } else {
        symbol
    };
    cell.set_symbol(symbol).set_style(gutter.style);
}

// #NOTE: Every note occupies at least one row and one spacer.
fn max_visible_notes(height: usize) -> usize {
    height.div_ceil(2)
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct NoteAnchor<'a> {
    content_id: &'a str,
    start: usize,
    end: usize,
}

struct NoteRows<'a> {
    bounds: BTreeMap<&'a str, Range<usize>>,
    rows: BTreeMap<NoteAnchor<'a>, Option<usize>>,
}

impl<'a> NoteRows<'a> {
    fn new(model: &'a ReaderModel) -> Self {
        let mut bounds = BTreeMap::<&str, Range<usize>>::new();
        for offset in 0..model.reading_height() {
            let Some(visible) = model.visible_row(offset) else {
                continue;
            };
            let cells = visible.document.layout.cells_on_row(visible.row);
            let Some(first) = cells.first() else {
                continue;
            };
            let range = cells.iter().fold(first.start..first.end, |range, cell| {
                range.start.min(cell.start)..range.end.max(cell.end)
            });
            bounds
                .entry(&visible.document.parsed.source.content_id)
                .and_modify(|existing| {
                    existing.start = existing.start.min(range.start);
                    existing.end = existing.end.max(range.end);
                })
                .or_insert(range);
        }
        Self {
            bounds,
            rows: BTreeMap::new(),
        }
    }

    fn row(
        &mut self,
        model: &ReaderModel,
        anchor: &'a crate::annotations::TextAnchor,
    ) -> Option<usize> {
        if !self
            .bounds
            .get(anchor.content_id())
            .is_some_and(|range| anchor.start() < range.end && anchor.end() > range.start)
        {
            return None;
        }
        let key = NoteAnchor {
            content_id: anchor.content_id(),
            start: anchor.start(),
            end: anchor.end(),
        };
        *self
            .rows
            .entry(key)
            .or_insert_with(|| note_row(model, anchor))
    }
}

fn note_row(model: &ReaderModel, anchor: &crate::annotations::TextAnchor) -> Option<usize> {
    (0..model.reading_height()).rev().find(|offset| {
        model.visible_row(*offset).is_some_and(|visible| {
            visible.document.parsed.source.content_id == anchor.content_id()
                && visible
                    .document
                    .layout
                    .cells_on_row(visible.row)
                    .iter()
                    .any(|cell| cell.start < anchor.end() && cell.end > anchor.start())
        })
    })
}

pub(in crate::terminal) fn draw_overlay(
    frame: &mut Frame<'_>,
    model: &ReaderModel,
    style: Style,
    reading_x: usize,
) {
    let ReaderMode::Annotations(mode) = model.view().mode else {
        return;
    };
    if let Some(editor) = mode.editor()
        && !matches!(mode, AnnotationMode::QuitNote(_))
    {
        draw_editor(frame, editor, style);
    }
    match mode {
        AnnotationMode::List { selected } => {
            let labels = model
                .view()
                .info
                .annotations
                .iter()
                .map(|annotation| {
                    format!(
                        "{}: {} | {}",
                        annotation.chapter,
                        annotation.anchor.quote().replace('\n', " "),
                        if annotation.note.is_empty() {
                            "(No note)"
                        } else {
                            &annotation.note
                        }
                    )
                })
                .collect::<Vec<_>>();
            draw_scrolled_list(
                frame,
                ListPaint {
                    title: "Annotations | Enter jump  e edit  d delete  f read  r copy  Esc close",
                    labels: &labels,
                    selected: *selected,
                    first: model.view().list_top,
                    empty_message: Some(
                        "No annotations yet. Press v while reading to select text.",
                    ),
                    style,
                },
            );
        }
        AnnotationMode::Choosing(chooser) => {
            let labels = chooser
                .records
                .iter()
                .map(|annotation| {
                    format!(
                        "{} | {}",
                        annotation.anchor.quote().replace('\n', " "),
                        annotation.note
                    )
                })
                .collect::<Vec<_>>();
            draw_scrolled_list(
                frame,
                ListPaint {
                    title: "Overlapping highlights | Enter select  Esc cancel",
                    labels: &labels,
                    selected: chooser.selected,
                    first: model.view().list_top,
                    empty_message: None,
                    style,
                },
            );
        }
        AnnotationMode::Viewing(viewer) => {
            let area = popup(frame.area());
            let lines = note_view_lines(
                viewer.annotation.anchor.quote(),
                &viewer.annotation.note,
                area.width.saturating_sub(2) as usize,
            );
            let block = Block::default()
                .title("Quotation and note | j/k scroll  Esc return")
                .borders(Borders::ALL)
                .style(style);
            let inside = block.inner(area);
            frame.render_widget(Clear, area);
            frame.render_widget(block, area);
            let first = viewer
                .scroll
                .min(lines.len().saturating_sub(inside.height as usize));
            let text = lines
                .iter()
                .skip(first)
                .take(inside.height as usize)
                .map(|line| safe_text(line))
                .collect::<Vec<_>>()
                .join("\n");
            frame.render_widget(Paragraph::new(text).style(style), inside);
        }
        AnnotationMode::QuitNote(_) => draw_list(
            frame,
            "Discard unsaved note and quit?",
            &["y: quit without saving    any other key: keep the draft".into()],
            0,
            style,
        ),
        AnnotationMode::Confirming(confirmation) => draw_list(
            frame,
            "Confirmation",
            &[
                confirmation.message.clone(),
                "y: confirm    any other key: cancel".into(),
            ],
            1,
            style,
        ),
        AnnotationMode::Pending(pending) if pending.error.is_some() => {
            let label = format!(
                "{} | Enter retries; Escape returns to the unsaved draft",
                pending.error.as_deref().unwrap_or_default()
            );
            let area = frame.area();
            frame.render_widget(
                Paragraph::new(clip_text(&safe_text(&label), area.width as usize))
                    .style(style.add_modifier(Modifier::REVERSED)),
                Rect::new(0, area.height.saturating_sub(2), area.width, 1),
            );
        }
        AnnotationMode::Selecting(selection) => {
            if let Some(cell) = selection.cell(model.selection_surface()) {
                let row = cell.row.saturating_sub(model.selection_top());
                let col = reading_x + cell.col;
                if row < frame.area().height.saturating_sub(1) as usize
                    && col < frame.area().width as usize
                {
                    frame.set_cursor_position((col as u16, row as u16));
                }
            }
        }
        _ => {}
    }
}

fn draw_editor(frame: &mut Frame<'_>, editor: &NoteEditor, style: Style) {
    let area = frame.area();
    let height = area.height.saturating_sub(2).min(11);
    let top = area.height.saturating_sub(height + 1);
    let rectangle = Rect::new(
        1.min(area.width),
        top,
        area.width.saturating_sub(2).min(72),
        height,
    );
    frame.render_widget(Clear, rectangle);
    let block = Block::default()
        .title("Note | Enter newline  Tab controls  Esc cancel")
        .borders(Borders::ALL)
        .style(style);
    let inside = block.inner(rectangle);
    frame.render_widget(block, rectangle);
    let width = inside.width.clamp(1, 70);
    let layout = editor.state.layout(width as usize);
    let visible = inside.height.saturating_sub(1).max(1) as usize;
    let first = draw_rows(
        frame,
        EditorRows {
            layout: &layout,
            area: Rect::new(inside.x, inside.y, width, inside.height),
            visible_rows: visible,
            clipping: TextClipping::Graphemes,
            style,
        },
    );
    let mut col = inside.x;
    let button_row = inside.y.saturating_add(inside.height.saturating_sub(1));
    for (index, action) in editor.state.buttons().iter().enumerate() {
        let label = format!("[ {} ]", action.label());
        if col + label.len() as u16 > inside.x + inside.width {
            if editor.state.focus() != index + 1 {
                continue;
            }
            col = inside.x;
        }
        let button_style = if editor.state.focus() == index + 1 {
            style.add_modifier(Modifier::REVERSED)
        } else {
            style
        };
        if button_row < area.height {
            frame.render_widget(
                Paragraph::new(clip_text(&label, inside.width as usize)).style(button_style),
                Rect::new(
                    col,
                    button_row,
                    label.len().min(inside.width as usize) as u16,
                    1,
                ),
            );
        }
        col = col.saturating_add(label.len() as u16 + 3);
    }
    if editor.state.focus() == 0 && inside.width > 0 && inside.height > 0 {
        frame.set_cursor_position((
            inside.x + (layout.cursor_col as u16).min(inside.width - 1),
            inside.y
                + (layout.cursor_row.saturating_sub(first) as u16)
                    .min(inside.height.saturating_sub(2)),
        ));
    }
}
