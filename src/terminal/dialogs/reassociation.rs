use crate::app::modes::reassociation::ReassociationMode;
use crate::app::{ReaderMode, ReaderModel};
use crate::terminal::widgets::list::{ListPaint, draw_list, draw_scrolled_list};
use ratatui::Frame;
use ratatui::style::Style;

pub(in crate::terminal) fn draw(frame: &mut Frame<'_>, model: &ReaderModel, style: Style) {
    if let ReaderMode::Reassociation(mode) = model.view().mode {
        draw_mode(frame, mode, model.view().list_top, style);
    }
}

fn draw_mode(frame: &mut Frame<'_>, mode: &ReassociationMode, first: usize, style: Style) {
    match mode {
        ReassociationMode::Sources(picker) => {
            let labels = picker
                .sources
                .iter()
                .map(|source| {
                    format!(
                        "{} | {} | {} annotations | {}",
                        if source.book.title.is_empty() {
                            "Untitled book"
                        } else {
                            &source.book.title
                        },
                        source.book.author,
                        source.annotation_count,
                        source
                            .book
                            .fingerprint
                            .get(..12)
                            .unwrap_or(&source.book.fingerprint)
                    )
                })
                .collect::<Vec<_>>();
            draw_scrolled_list(
                frame,
                ListPaint {
                    title: "Choose source identity | j/k choose  Enter confirm  Esc return",
                    labels: &labels,
                    selected: picker.selected,
                    first,
                    empty_message: None,
                    style,
                },
            );
        }
        ReassociationMode::Confirm(picker) => {
            let labels = picker
                .sources
                .get(picker.selected)
                .map_or_else(Vec::new, |source| {
                    vec![
                        source.book.title.clone(),
                        format!(
                            "Validate {} source annotations against this book.",
                            source.annotation_count
                        ),
                        "Only exact document positions and quotations are copied.".into(),
                        "Original annotations, export links and files remain unchanged.".into(),
                        "y: copy validated annotations    any other key: cancel".into(),
                    ]
                });
            draw_list(
                frame,
                "Copy validated annotations from this identity?",
                &labels,
                labels.len().saturating_sub(1),
                style,
            );
        }
        ReassociationMode::Pending(pending) => {
            if let Some(error) = &pending.error {
                draw_list(
                    frame,
                    "Reassociation failed",
                    &[
                        error.clone(),
                        "Enter: retry    Escape: return without copying".into(),
                    ],
                    1,
                    style,
                );
            } else {
                draw_mode(frame, &pending.resume, first, style);
            }
        }
    }
}
