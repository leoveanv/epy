use super::CleanupSequences;
use super::image_preparation::ImagePreparation;
use super::images::{ImageIds, ImageSupport};
use super::render::page_panes;
use crate::app::modes::images::ImageMode;
use crate::app::{ReaderMode, ReaderModel};
use crate::book::BookLocation;
use crate::images::{
    ImageEncoding, ImageJob, ImageProtocol, ImageTarget, ImageWorker, ImageWorkerOptions,
    PreparedInlineImage,
};
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui_image::sliced::{SignedPosition, SlicedImage};
use std::cell::Cell;
use std::io::Write;

const IMAGES_MAX: usize = 32;
const PREPARED_BYTES_MAX: usize = 96 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
struct InlineGeometry {
    columns: u16,
    rows: u16,
    support: ImageSupport,
}

impl InlineGeometry {
    fn encoding(self, image_id: u32) -> ImageEncoding {
        ImageEncoding {
            columns: self.columns,
            rows: self.rows,
            cell_width_px: self.support.cell_size.width,
            cell_height_px: self.support.cell_size.height,
            protocol: if self.support.kitty {
                ImageProtocol::Kitty {
                    image_id,
                    in_tmux: self.support.in_tmux,
                }
            } else {
                ImageProtocol::Halfblocks
            },
        }
    }

    // #NOTE: Reserve conservative protocol storage before retaining an image; visible slots cannot grow the encoded cache without a bound.
    fn reserved_bytes(self) -> usize {
        if self.support.kitty {
            let width = (usize::from(self.columns.min(296))
                * usize::from(self.support.cell_size.width))
            .min(2048);
            let height = (usize::from(self.rows.min(296))
                * usize::from(self.support.cell_size.height))
            .min(2048);
            width * height * 8 + 8192
        } else {
            usize::from(self.columns.min(2048))
                * usize::from(self.rows.min(2048))
                * std::mem::size_of::<ratatui_image::protocol::halfblocks::HalfBlock>()
                * 2
                + 8192
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct InlineAsset {
    book: String,
    content_id: String,
    path: String,
    geometry: InlineGeometry,
}

struct InlinePlacement {
    asset: InlineAsset,
    row_offset: isize,
}

struct InlineEntry {
    asset: InlineAsset,
    result: Result<PreparedInlineImage, String>,
    painted: Cell<bool>,
}

impl InlineEntry {
    fn new(asset: InlineAsset, result: Result<PreparedInlineImage, String>) -> Self {
        Self {
            asset,
            result,
            painted: Cell::new(false),
        }
    }
}

struct LoadingImage {
    asset: InlineAsset,
    generation: u64,
    request_id: u64,
}

struct PreparingImage {
    asset: InlineAsset,
    version: u64,
}

enum PendingImage {
    Loading(LoadingImage),
    Preparing(PreparingImage),
}

impl PendingImage {
    fn asset(&self) -> &InlineAsset {
        match self {
            Self::Loading(image) => &image.asset,
            Self::Preparing(image) => &image.asset,
        }
    }
}

pub(in crate::terminal) struct InlineImages {
    options: ImageWorkerOptions,
    support: ImageSupport,
    worker: Option<ImageWorker>,
    preparation: Option<ImagePreparation>,
    pending: Option<PendingImage>,
    entries: Vec<InlineEntry>,
    placements: Vec<InlinePlacement>,
    image_ids: ImageIds,
    next_request: u64,
    cleanup: CleanupSequences,
}

impl InlineImages {
    pub fn new(options: ImageWorkerOptions, support: ImageSupport, image_ids: ImageIds) -> Self {
        Self {
            options,
            support,
            worker: None,
            preparation: None,
            pending: None,
            entries: Vec::new(),
            placements: Vec::new(),
            cleanup: CleanupSequences::default(),
            image_ids,
            next_request: 0,
        }
    }

    pub fn set_support(&mut self, support: ImageSupport) {
        self.support = support;
    }

    pub fn sync(
        &mut self,
        model: Option<&ReaderModel>,
        size: Size,
        location: &BookLocation,
    ) -> bool {
        let desired = model
            .filter(|model| can_paint(model))
            .map_or_else(Vec::new, |model| placements(model, size, self.support));
        let changed = !same_placements(&self.placements, &desired);
        self.placements = desired;
        self.retire_invisible();
        let completed = self.complete();
        self.request_next(location);
        changed || completed
    }

    fn retire_invisible(&mut self) {
        if self.pending.as_ref().is_some_and(|pending| {
            !self
                .placements
                .iter()
                .any(|placement| &placement.asset == pending.asset())
        }) {
            self.cancel_pending();
        }
        let placements = &self.placements;
        let cleanup = &mut self.cleanup;
        self.entries.retain(|entry| {
            let retained = placements
                .iter()
                .any(|placement| placement.asset == entry.asset);
            if !retained
                && let Ok(image) = &entry.result
                && let Some(sequence) = image.cleanup_sequence()
            {
                cleanup.push(sequence);
            }
            retained
        });
    }

    fn retire(&mut self, entry: InlineEntry) {
        if let Ok(image) = entry.result
            && let Some(sequence) = image.cleanup_sequence()
        {
            self.cleanup.push(sequence);
        }
    }

    fn complete(&mut self) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        match pending {
            PendingImage::Loading(loading) => {
                let Some(result) = self.worker.as_ref().and_then(ImageWorker::take_result) else {
                    self.pending = Some(PendingImage::Loading(loading));
                    return false;
                };
                if result.generation != loading.generation
                    || result.request_id != loading.request_id
                {
                    self.pending = Some(PendingImage::Loading(loading));
                    return false;
                }
                match result.result {
                    Ok(payload) => {
                        let image_id = self.image_ids.allocate();
                        let version = self
                            .preparation
                            .get_or_insert_with(ImagePreparation::start)
                            .request(payload, loading.asset.geometry.encoding(image_id));
                        self.pending = Some(PendingImage::Preparing(PreparingImage {
                            asset: loading.asset,
                            version,
                        }));
                    }
                    Err(error) => self
                        .entries
                        .push(InlineEntry::new(loading.asset, Err(error))),
                }
            }
            PendingImage::Preparing(preparing) => {
                let Some(result) = self
                    .preparation
                    .as_ref()
                    .and_then(ImagePreparation::take_result)
                else {
                    self.pending = Some(PendingImage::Preparing(preparing));
                    return false;
                };
                if result.version != preparing.version {
                    self.pending = Some(PendingImage::Preparing(preparing));
                    return false;
                }
                let result = result
                    .result
                    .and_then(|image| image.into_sliced().map_err(|error| error.to_string()));
                self.entries.push(InlineEntry::new(preparing.asset, result));
            }
        }
        true
    }

    fn request_next(&mut self, location: &BookLocation) {
        if self.pending.is_some() {
            return;
        }
        let Some(asset) = self
            .placements
            .iter()
            .map(|placement| &placement.asset)
            .find(|asset| !self.entries.iter().any(|entry| &entry.asset == *asset))
            .cloned()
        else {
            return;
        };
        let reserved = self
            .entries
            .iter()
            .filter(|entry| entry.result.is_ok())
            .map(|entry| entry.asset.geometry.reserved_bytes())
            .sum::<usize>();
        if reserved.saturating_add(asset.geometry.reserved_bytes()) > PREPARED_BYTES_MAX {
            self.entries.push(InlineEntry::new(
                asset,
                Err("Inline image budget exceeded; open the image preview instead.".into()),
            ));
            return;
        }
        let options = self.options.clone();
        let worker = self
            .worker
            .get_or_insert_with(|| ImageWorker::start(options));
        self.next_request = self.next_request.wrapping_add(1);
        let generation = worker.request(
            location.clone(),
            ImageJob {
                request_id: self.next_request,
                target: ImageTarget {
                    content_id: asset.content_id.clone(),
                    path: asset.path.clone(),
                    row_offset: 0,
                },
            },
        );
        self.pending = Some(PendingImage::Loading(LoadingImage {
            asset,
            generation,
            request_id: self.next_request,
        }));
    }

    fn cancel_pending(&mut self) {
        self.pending = None;
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        if let Some(preparation) = &self.preparation {
            preparation.cancel();
        }
    }

    pub fn clear(&mut self) {
        self.cancel_pending();
        self.placements.clear();
        for entry in std::mem::take(&mut self.entries) {
            self.retire(entry);
        }
    }

    pub fn flush_cleanup(&mut self, output: &mut impl Write) -> std::io::Result<()> {
        self.cleanup.flush(output)
    }

    pub fn draw(&self, frame: &mut Frame<'_>, model: &ReaderModel) {
        if !can_paint(model) {
            return;
        }
        let panes = page_panes(model, frame.area());
        for placement in &self.placements {
            let Some(entry) = self
                .entries
                .iter()
                .find(|entry| entry.asset == placement.asset)
            else {
                continue;
            };
            if entry.asset.geometry.support.kitty && !entry.painted.get() && !can_upload(model) {
                continue;
            }
            let Ok(image) = &entry.result else {
                continue;
            };
            for (index, pane) in panes.iter().enumerate() {
                let offset = placement.row_offset.saturating_sub(
                    isize::try_from(index * model.view().viewport.rows).unwrap_or(isize::MAX),
                );
                if draw_image(frame, image, pane.area, offset) {
                    entry.painted.set(true);
                }
            }
        }
    }
}

impl Drop for InlineImages {
    fn drop(&mut self) {
        self.clear();
        let _ = self.flush_cleanup(&mut std::io::stdout());
    }
}

fn can_paint(model: &ReaderModel) -> bool {
    model.view().configuration.settings.inline_images
        && !matches!(model.view().mode, ReaderMode::Images(mode) if !matches!(mode.as_ref(), ImageMode::Selecting(_)))
}

// #NOTE: A text overlay can erase a first transmission from the frame buffer; defer new Kitty uploads until that overlay closes.
fn can_upload(model: &ReaderModel) -> bool {
    match model.view().mode {
        ReaderMode::Reading | ReaderMode::Quickmark { .. } => true,
        ReaderMode::Images(mode) => matches!(mode.as_ref(), ImageMode::Selecting(_)),
        _ => false,
    }
}

fn placements(model: &ReaderModel, size: Size, support: ImageSupport) -> Vec<InlinePlacement> {
    let panes = page_panes(model, Rect::new(0, 0, size.width, size.height));
    let columns = panes.first().map_or(0, |pane| pane.area.width);
    if columns == 0 {
        return Vec::new();
    }
    let geometry = InlineGeometry {
        columns,
        rows: model
            .view()
            .rendered
            .layout
            .text_lines
            .image_layout()
            .height() as u16,
        support,
    };
    model
        .visible_images()
        .take(IMAGES_MAX)
        .map(|target| InlinePlacement {
            asset: InlineAsset {
                book: model.view().info.path.clone(),
                content_id: target.content_id,
                path: target.path,
                geometry,
            },
            row_offset: target.row_offset,
        })
        .collect()
}

fn same_placements(left: &[InlinePlacement], right: &[InlinePlacement]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.asset == right.asset && left.row_offset == right.row_offset)
}

fn draw_image(
    frame: &mut Frame<'_>,
    image: &PreparedInlineImage,
    pane: Rect,
    row_offset: isize,
) -> bool {
    let size = image.protocol.size();
    if row_offset >= isize::try_from(pane.height).unwrap_or(isize::MAX)
        || row_offset.saturating_add(isize::try_from(size.height).unwrap_or(isize::MAX)) <= 0
    {
        return false;
    }
    let top = row_offset.max(0) as u16;
    let area = Rect::new(
        pane.x,
        pane.y.saturating_add(top),
        pane.width,
        pane.height.saturating_sub(top).min(size.height),
    )
    .intersection(frame.area());
    if area.is_empty() {
        return false;
    }
    let x = area.width.saturating_sub(size.width) / 2;
    let position = SignedPosition::from((x as i16, row_offset.min(0) as i16));
    frame.render_widget(SlicedImage::new(&image.protocol, position), area);
    true
}
