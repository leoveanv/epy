use crate::app::editor::EditorState;
use crate::app::modes::markdown_export::LinkMode;
use crate::app::{ReaderMode, ReaderModel};
use crate::document::text::clip_text;
use crate::terminal::widgets::editor::{EditorRows, TextClipping, draw_rows};
use crate::terminal::widgets::list::{draw_list, popup};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub(in crate::terminal) fn draw(frame: &mut Frame<'_>, model: &ReaderModel, style: Style) {
    if let ReaderMode::Link(mode) = model.view().mode {
        draw_mode(frame, model, mode, style);
    }
}

fn draw_mode(frame: &mut Frame<'_>, model: &ReaderModel, mode: &LinkMode, style: Style) {
    match mode {
        LinkMode::Summary => {
            let mut lines = vec![
                model
                    .export_state()
                    .link
                    .as_ref()
                    .map_or("No Markdown destination".into(), |link| link.path.clone()),
            ];
            lines.push(if let Some(error) = &model.export_state().error {
                format!("Export pending: {error}")
            } else if model.export_state().pending() {
                "Export queued or in progress".into()
            } else {
                "Export up to date".into()
            });
            lines.extend([
                "r: retry export    w: confirm overwrite of external edits".into(),
                "Enter: change or recreate destination    u: unlink    Esc: close".into(),
                "Annotations remain saved locally if export fails.".into(),
            ]);
            draw_list(frame, "Markdown link", &lines, 0, style);
        }
        LinkMode::Editing(editor) => draw_path_editor(frame, editor, style),
        LinkMode::Destination { destination, .. } => draw_list(
            frame,
            "Confirm resolved Markdown destination",
            &[
                destination.path.clone(),
                "y: approve destination    any other key: edit path".into(),
            ],
            1,
            style,
        ),
        LinkMode::Create { destination, .. } => draw_list(
            frame,
            "Create missing Markdown file?",
            &[
                destination.path.clone(),
                "y: create file    any other key: edit path".into(),
            ],
            1,
            style,
        ),
        LinkMode::Unlink => draw_list(
            frame,
            "Remove this Markdown link?",
            &[
                "Annotations and Markdown files remain unchanged.".into(),
                "y: remove link    any other key: cancel".into(),
            ],
            1,
            style,
        ),
        LinkMode::Overwrite(approval) => draw_list(
            frame,
            "Overwrite external edits in the managed section?",
            &[
                approval.link.path.clone(),
                "Handwritten content outside the managed section remains unchanged.".into(),
                "Malformed or missing delimiters still block export.".into(),
                "y: confirm overwrite    any other key: cancel".into(),
            ],
            3,
            style,
        ),
        LinkMode::Pending(pending) => {
            if let Some(error) = &pending.error {
                draw_list(
                    frame,
                    "Markdown operation failed",
                    &[
                        error.clone(),
                        "Enter: retry    Escape: return without losing the path draft".into(),
                    ],
                    1,
                    style,
                );
            } else {
                draw_mode(frame, model, &pending.resume, style);
            }
        }
    }
}

fn draw_path_editor(frame: &mut Frame<'_>, editor: &EditorState, style: Style) {
    let area = popup(frame.area());
    let block = Block::default()
        .title("Markdown path | absolute .md path | Ctrl-U clear | Tab Save | Esc cancel")
        .borders(Borders::ALL)
        .style(style);
    let inside = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let layout = editor.layout(inside.width.max(1) as usize);
    let height = inside.height.saturating_sub(2) as usize;
    let first = draw_rows(
        frame,
        EditorRows {
            layout: &layout,
            area: inside,
            visible_rows: height,
            clipping: TextClipping::Widget,
            style,
        },
    );
    if inside.height > 0 {
        let labels = editor
            .buttons()
            .iter()
            .enumerate()
            .map(|(index, action)| {
                if editor.focus() == index + 1 {
                    format!("[{}]", action.label())
                } else {
                    action.label().into()
                }
            })
            .collect::<Vec<_>>()
            .join("    ");
        frame.render_widget(
            Paragraph::new(clip_text(&labels, inside.width as usize))
                .style(style.add_modifier(Modifier::BOLD)),
            Rect::new(inside.x, inside.y + inside.height - 1, inside.width, 1),
        );
    }
    if editor.focus() == 0 && height > 0 && inside.width > 0 {
        frame.set_cursor_position((
            inside.x + (layout.cursor_col as u16).min(inside.width - 1),
            inside.y + layout.cursor_row.saturating_sub(first) as u16,
        ));
    }
}
