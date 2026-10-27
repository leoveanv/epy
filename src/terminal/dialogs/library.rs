use super::render::safe_text;
use crate::app::modes::library::LibraryPhase;
use crate::app::{ReaderMode, ReaderModel};
use crate::book::LibraryEntry;
use crate::document::text::clip_text;
use crate::terminal::widgets::list::{ListRows, dialog_window, draw_rows};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Paragraph, Wrap};

fn label(entry: &LibraryEntry) -> String {
    let progress = entry.reading_progress.map_or_else(
        || "…".into(),
        |progress| format!("{:.0}%", progress * 100.0),
    );
    format!(
        "{progress:>4}  {}  |  {}  |  {}",
        entry.title.as_deref().unwrap_or("Untitled"),
        entry.author.as_deref().unwrap_or("Unknown author"),
        entry.filepath
    )
}

pub(in crate::terminal) fn draw(frame: &mut Frame<'_>, model: &ReaderModel, style: Style) {
    let ReaderMode::Library(mode) = model.view().mode else {
        return;
    };
    let inside = dialog_window(frame, "Library", style);
    let message = match &mode.phase {
        LibraryPhase::ConfirmRemove => Some(format!(
            "Remove {} from the library?\n\nThe saved position and bookmarks will be removed. The ebook, annotations, and Markdown exports remain.\n\ny: remove   n/Escape: cancel",
            mode.entries
                .get(mode.selected)
                .map_or_else(String::new, label)
        )),
        LibraryPhase::Opening(path) => Some(format!(
            "Opening {}…\n\nEscape cancels and keeps the current book open.",
            path
        )),
        LibraryPhase::Waiting { .. } => {
            Some("Saving position and updating library…\n\nEscape cancels the dialog.".into())
        }
        LibraryPhase::Browsing if mode.entries.is_empty() => {
            Some("No reading history. Escape closes.".into())
        }
        LibraryPhase::Browsing => None,
    };
    if let Some(message) = message {
        frame.render_widget(
            Paragraph::new(safe_text_lines(&message))
                .style(style)
                .wrap(Wrap { trim: false }),
            inside,
        );
        return;
    }
    draw_rows(
        frame,
        ListRows {
            area: Rect::new(
                inside.x,
                inside.y,
                inside.width,
                inside.height.saturating_sub(1),
            ),
            selected: mode.selected,
            first: model.view().list_top,
            style,
        },
        mode.entries.iter().map(label),
    );
    if inside.height > 0 {
        frame.render_widget(
            Paragraph::new(clip_text(
                "Enter: open   d: remove   Escape/R: close",
                inside.width as usize,
            ))
            .style(style),
            Rect::new(inside.x, inside.y + inside.height - 1, inside.width, 1),
        );
    }
}

fn safe_text_lines(value: &str) -> String {
    value
        .split('\n')
        .map(safe_text)
        .collect::<Vec<_>>()
        .join("\n")
}
