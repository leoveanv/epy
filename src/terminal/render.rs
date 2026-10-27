use super::annotation_render;
use super::highlights::Highlights;
use super::hyperlink_cues::VisibleLinks;
use super::widgets::list::{ListRows, dialog_window, draw_rows};
use crate::annotations::{ReadingLayout, reading_layout};
use crate::app::{ReaderMode, ReaderModel};
use crate::document::layout::FontStyle;
use crate::document::text::{cell_width, clip_text, graphemes};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Paragraph;

fn color(value: i64) -> Color {
    u8::try_from(value).map_or(Color::Reset, Color::Indexed)
}

pub(super) fn base_style(model: &ReaderModel) -> Style {
    let settings = &model.view().configuration.settings;
    let (foreground, background) = match model.view().color_scheme {
        1 => (settings.dark_color_fg, settings.dark_color_bg),
        2 => (settings.light_color_fg, settings.light_color_bg),
        _ => (settings.default_color_fg, settings.default_color_bg),
    };
    Style::default().fg(color(foreground)).bg(color(background))
}

pub(super) fn safe_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

pub(super) fn draw_small_terminal(frame: &mut Frame<'_>) {
    frame.render_widget(
        Paragraph::new(format!(
            "Terminal too small. Resize to at least {}x{}.",
            super::MINIMUM_COLUMNS,
            super::MINIMUM_ROWS
        )),
        frame.area(),
    );
}

pub(super) fn draw(
    frame: &mut Frame<'_>,
    model: Option<&ReaderModel>,
    inline: Option<&super::inline_images::InlineImages>,
) {
    let area = frame.area();
    let Some(model) = model else {
        frame.render_widget(Paragraph::new("Opening ebook…"), area);
        return;
    };
    let style = base_style(model);
    frame.buffer_mut().set_style(area, style);
    let geometry = reading_geometry(model, area.width as usize);
    let left = geometry.reading_x;
    let visible = (0..model.reading_height() * model.view().spread)
        .filter_map(|offset| model.visible_row(offset))
        .collect::<Vec<_>>();
    let highlights = Highlights::visible(&visible, &model.view().info.annotations);
    let links = VisibleLinks::new(&visible);
    for column in 0..model.view().spread {
        for screen_row in 0..model
            .reading_height()
            .min(area.height.saturating_sub(1) as usize)
        {
            let Some(visible) = visible.get(column * model.view().viewport.rows + screen_row)
            else {
                continue;
            };
            let row = visible.row;
            let Some(line) = visible.document.layout.text_lines.get(row) else {
                continue;
            };
            draw_row(
                frame,
                RowPaint {
                    model,
                    document: visible.document,
                    row,
                    line,
                    x: (left + column * (model.view().width + 7)) as u16,
                    y: screen_row as u16,
                    base: style,
                    highlights: &highlights,
                    links: &links,
                },
            );
        }
    }
    if let Some(inline) = inline {
        inline.draw(frame, model);
    }
    annotation_render::draw_notes(frame, model, geometry, style);
    let status = match model.view().mode {
        ReaderMode::BookmarkPrompt(value) if model.view().input_warning => {
            format!("{} Bookmark: {value}", model.view().status)
        }
        ReaderMode::BookmarkPrompt(value) => format!("Bookmark: {value}"),
        _ if !model.view().status.is_empty() => model.view().status.to_owned(),
        _ if super::hyperlinks::status_hint(model).is_some() => {
            super::hyperlinks::status_hint(model).unwrap_or_default()
        }
        _ if super::search_render::status_hint(model).is_some() => {
            super::search_render::status_hint(model).unwrap_or_default()
        }
        _ if annotation_render::status_hint(model).is_some() => {
            annotation_render::status_hint(model).unwrap_or_default()
        }
        _ if model.export_state().pending() => {
            "Annotations saved; Markdown export pending. Press O for recovery.".into()
        }
        _ if model.view().show_progress => format!(
            "{}  {}/{}  {}  row {}",
            model.view().info.metadata.title.as_deref().unwrap_or("ers"),
            model.view().rendered.index + 1,
            model.view().info.contents.len(),
            model.progress().map_or_else(
                || "…".into(),
                |progress| format!("{:.0}%", progress * 100.0)
            ),
            model.view().row + 1
        ),
        _ => String::new(),
    };
    let status = if model.view().status.is_empty() && !model.export_state().pending() {
        match links.status_hint(model) {
            Some(hint) if status.is_empty() => hint,
            Some(hint) => format!("{hint}  {status}"),
            None => status,
        }
    } else {
        status
    };
    frame.render_widget(
        Paragraph::new(clip_text(&safe_text(&status), area.width as usize)).style(style),
        Rect::new(0, area.height.saturating_sub(1), area.width, 1),
    );
    super::hyperlinks::draw(frame, model, style, left);
    annotation_render::draw_overlay(frame, model, style, left);
    super::link_render::draw(frame, model, style);
    super::reassociation_render::draw(frame, model, style);
    super::search_render::draw(frame, model, style);
    super::dictionary_render::draw(frame, model, style);
    super::library_render::draw(frame, model, style);
    if let ReaderMode::Dialog(dialog) = model.view().mode {
        let inside = dialog_window(frame, &dialog.title, style);
        draw_rows(
            frame,
            ListRows {
                area: inside,
                selected: dialog.selected,
                first: model.view().list_top,
                style,
            },
            dialog.items.iter().map(|item| &item.label),
        );
    }
}

fn reading_geometry(model: &ReaderModel, columns: usize) -> ReadingLayout {
    if model.view().spread == 1 {
        reading_layout(
            columns,
            model.view().width,
            model.view().configuration.settings.annotation_notes_width as usize,
        )
    } else {
        ReadingLayout {
            reading_x: columns.saturating_sub(model.view().width * model.view().spread + 7) / 2,
            notes_x: None,
            notes_width: 0,
        }
    }
}

pub(super) fn page_panes(
    model: &ReaderModel,
    area: Rect,
) -> Vec<crate::terminal::page_animation::PagePane> {
    let geometry = reading_geometry(model, area.width as usize);
    let style = base_style(model);
    let height = model
        .reading_height()
        .min(area.height.saturating_sub(1) as usize) as u16;
    (0..model.view().spread)
        .map(|column| crate::terminal::page_animation::PagePane {
            area: Rect::new(
                area.x.saturating_add(
                    (geometry.reading_x + column * (model.view().width + 7)) as u16,
                ),
                area.y,
                model.view().width as u16,
                height,
            ),
            style,
        })
        .collect()
}

struct RowPaint<'a> {
    model: &'a ReaderModel,
    document: &'a crate::document::RenderedDocument,
    row: usize,
    line: &'a str,
    x: u16,
    y: u16,
    base: Style,
    highlights: &'a Highlights<'a>,
    links: &'a VisibleLinks<'a>,
}

fn draw_row(frame: &mut Frame<'_>, paint: RowPaint<'_>) {
    let RowPaint {
        model,
        document,
        row,
        line,
        x,
        y,
        base,
        highlights,
        links,
    } = paint;
    let line_width = cell_width(line);
    let active_document = document.index == model.view().rendered.index;
    let zero_marker = if active_document {
        super::search_render::zero_marker(model, row, line_width)
    } else {
        None
    };
    let mut col = 0;
    let mut character_offset = 0;
    for cluster in graphemes(line) {
        let width = cell_width(cluster);
        let mut style = base;
        for span in document.styles_on_row(row).iter().filter(|span| {
            span.row == row
                && character_offset >= span.col
                && character_offset < span.col + span.n_letters
        }) {
            style = style.add_modifier(match span.style {
                FontStyle::Bold => Modifier::BOLD,
                FontStyle::Italic => Modifier::ITALIC,
                FontStyle::Normal => Modifier::empty(),
            });
        }
        if let Some(cell) = document
            .layout
            .cells_on_row(row)
            .iter()
            .find(|cell| col >= cell.col && col < cell.col + cell.width)
        {
            if links.overlaps(document.index, cell.start..cell.end) {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if highlights.overlaps(&document.parsed.source.content_id, cell.start..cell.end) {
                style = style
                    .fg(color(
                        model.view().configuration.settings.annotation_color_fg,
                    ))
                    .bg(color(
                        model.view().configuration.settings.annotation_color_bg,
                    ));
            }
        }
        if let Some(selection) = model.active_selection()
            && model
                .selection_index(document.index, row, col)
                .is_some_and(|index| selection.selected_indices().contains(&index))
        {
            style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        }
        if active_document
            && (super::search_render::selected(model, row, col, width)
                || (col + width == line_width
                    && col + width >= model.view().width
                    && zero_marker.is_some()))
        {
            style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        }
        if width > 0 && x as usize + col + width <= frame.area().width as usize {
            frame
                .buffer_mut()
                .set_stringn(x + col as u16, y, safe_text(cluster), width, style);
        }
        col += width;
        character_offset += cluster.chars().count();
    }
    if let Some(column) = zero_marker
        && column < model.view().width
        && (x as usize + column) < frame.area().width as usize
    {
        frame.buffer_mut().set_stringn(
            x + column as u16,
            y,
            " ",
            1,
            base.add_modifier(Modifier::REVERSED | Modifier::BOLD),
        );
    }
}
