use super::CleanupSequences;
use super::image_preparation::ImagePreparation;
use super::render::{base_style, page_panes, safe_text};
use crate::app::modes::images::ImageMode;
use crate::app::{ReaderMode, ReaderModel};
use crate::images::{ImageEncoding, ImageProtocol, PreparedImage};
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::cell::Cell;
use std::io::Write;
use std::rc::Rc;

#[derive(Clone)]
pub(in crate::terminal) struct ImageIds {
    next: Rc<Cell<u32>>,
}

impl ImageIds {
    pub fn new(seed: u32) -> Self {
        Self {
            next: Rc::new(Cell::new(seed)),
        }
    }

    pub fn allocate(&self) -> u32 {
        let image_id = self.next.get().wrapping_add(1).max(1);
        self.next.set(image_id);
        image_id
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::terminal) struct ImageSupport {
    pub cell_size: Size,
    pub kitty: bool,
    pub in_tmux: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PreviewKey {
    generation: u64,
    size: Size,
    cell_size: Size,
    kitty: bool,
    in_tmux: bool,
}

pub(in crate::terminal) struct ImageRenderer {
    support: ImageSupport,
    preparation: Option<ImagePreparation>,
    requested: Option<PreviewKey>,
    version: u64,
    image_ids: ImageIds,
    ready: Option<PreparedImage>,
    error: Option<String>,
    cleanup: CleanupSequences,
}

impl ImageRenderer {
    pub fn new(support: ImageSupport, image_ids: ImageIds) -> Self {
        Self {
            support,
            preparation: None,
            requested: None,
            version: 0,
            image_ids,
            ready: None,
            error: None,
            cleanup: CleanupSequences::default(),
        }
    }

    pub fn support(&self) -> ImageSupport {
        self.support
    }

    pub fn set_support(&mut self, support: ImageSupport) {
        self.support = support;
    }

    pub fn sync(&mut self, model: Option<&ReaderModel>, size: Size) -> bool {
        let preview = model.and_then(|model| match model.view().mode {
            ReaderMode::Images(mode) => match mode.as_ref() {
                ImageMode::Preview(preview) if preview.payload.decoded.is_ok() => Some(preview),
                _ => None,
            },
            _ => None,
        });
        let desired = preview.map(|preview| PreviewKey {
            generation: preview.generation,
            size: preview_area(Rect::new(0, 0, size.width, size.height)).as_size(),
            cell_size: self.support.cell_size,
            kitty: self.support.kitty,
            in_tmux: self.support.in_tmux,
        });
        let mut changed = false;
        if desired != self.requested {
            self.clear();
            self.requested = desired;
            if let (Some(key), Some(preview)) = (desired, preview) {
                let image_id = self.image_ids.allocate();
                let protocol = if key.kitty {
                    ImageProtocol::Kitty {
                        image_id,
                        in_tmux: key.in_tmux,
                    }
                } else {
                    ImageProtocol::Halfblocks
                };
                self.version = self
                    .preparation
                    .get_or_insert_with(ImagePreparation::start)
                    .request(
                        preview.payload.clone(),
                        ImageEncoding {
                            columns: key.size.width,
                            rows: key.size.height,
                            cell_width_px: key.cell_size.width,
                            cell_height_px: key.cell_size.height,
                            protocol,
                        },
                    );
            }
            changed = true;
        }
        if let Some(result) = self
            .preparation
            .as_ref()
            .and_then(ImagePreparation::take_result)
            && result.version == self.version
            && self.requested.is_some()
        {
            match result.result {
                Ok(image) => self.ready = Some(image),
                Err(error) => self.error = Some(error),
            }
            changed = true;
        }
        changed
    }

    pub fn clear(&mut self) {
        if let Some(image) = self.ready.take()
            && let Some(sequence) = image.cleanup_sequence()
        {
            self.cleanup.push(sequence);
        }
        if let Some(preparation) = &self.preparation {
            preparation.cancel();
        }
        self.requested = None;
        self.error = None;
    }

    pub fn flush_cleanup(&mut self, output: &mut impl Write) -> std::io::Result<()> {
        self.cleanup.flush(output)
    }

    pub fn draw(&self, frame: &mut Frame<'_>, model: Option<&ReaderModel>) {
        let Some(model) = model else {
            return;
        };
        let ReaderMode::Images(mode) = model.view().mode else {
            return;
        };
        let style = base_style(model);
        if let ImageMode::Selecting(selection) = mode.as_ref() {
            if let Some(target) = selection.targets.get(selection.selected) {
                let offset = target.row_offset.max(0) as usize;
                let panes = page_panes(model, frame.area());
                if let Some(pane) = panes.get(offset / model.view().viewport.rows.max(1))
                    && pane.area.x > frame.area().x
                {
                    let marker = Rect::new(
                        pane.area.x - 1,
                        pane.area
                            .y
                            .saturating_add((offset % model.view().viewport.rows.max(1)) as u16),
                        1,
                        1,
                    )
                    .intersection(frame.area());
                    frame.render_widget(
                        Paragraph::new("▸").style(style.add_modifier(Modifier::BOLD)),
                        marker,
                    );
                }
            }
            draw_footer(
                frame,
                &format!(
                    "Image {}/{} | j/k or arrows: select | Enter: preview | Esc: cancel",
                    selection.selected + 1,
                    selection.targets.len()
                ),
                style,
            );
            return;
        }
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.buffer_mut().set_style(area, style);
        let body = preview_area(area);
        if area.width > 2 && area.height > 3 {
            frame.render_widget(
                Block::default().borders(Borders::ALL).style(style),
                Rect::new(1, 1, area.width - 2, area.height - 2),
            );
        }
        let title = match mode.as_ref() {
            ImageMode::Preview(preview) => {
                format!("Image preview | {}", safe_text(&preview.payload.name))
            }
            _ => "Image preview".into(),
        };
        frame.render_widget(
            Paragraph::new(title).style(style.add_modifier(Modifier::BOLD)),
            Rect::new(0, 0, area.width, area.height.min(1)),
        );
        match mode.as_ref() {
            ImageMode::Loading { .. } => draw_message(frame, body, "Loading image…", style),
            ImageMode::Failed(error) => draw_message(frame, body, error, style),
            ImageMode::Preview(preview) => {
                if let Err(error) = &preview.payload.decoded {
                    draw_message(frame, body, error, style);
                } else if let Some(error) = &self.error {
                    draw_message(frame, body, error, style);
                } else if let Some(image) = &self.ready {
                    let size = image.protocol.size();
                    let centered = Rect::new(
                        body.x + body.width.saturating_sub(size.width) / 2,
                        body.y + body.height.saturating_sub(size.height) / 2,
                        size.width.min(body.width),
                        size.height.min(body.height),
                    );
                    frame.render_widget(ratatui_image::Image::new(&image.protocol), centered);
                } else {
                    draw_message(frame, body, "Preparing image…", style);
                }
            }
            ImageMode::Selecting(_) => {}
        }
        draw_footer(
            frame,
            if model.view().status.is_empty() {
                "Esc/Enter: return | e: open externally"
            } else {
                model.view().status
            },
            style,
        );
    }
}

impl Drop for ImageRenderer {
    fn drop(&mut self) {
        self.clear();
        let _ = self.flush_cleanup(&mut std::io::stdout());
    }
}

fn preview_area(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(2),
        area.width.saturating_sub(4),
        area.height.saturating_sub(4),
    )
}

fn draw_message(frame: &mut Frame<'_>, area: Rect, message: &str, style: Style) {
    frame.render_widget(
        Paragraph::new(safe_text(message))
            .style(style)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, text: &str, style: Style) {
    let area = frame.area();
    if area.height > 0 {
        frame.render_widget(
            Paragraph::new(safe_text(text)).style(style),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
}
