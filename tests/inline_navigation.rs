#[path = "support/reader.rs"]
mod reader_support;

use ers::app::modes::images::ImageMode;
use ers::app::{ReaderEffect, ReaderMode, ReaderModel, ReaderSession, Viewport};
use ers::configuration::InputKey;
use ers::document::RenderedDocument;
use ers::document::layout::LayoutOptions;
use ers::document::text::parse_document;
use ers::images::{ImageJob, ImageResult};
use ers::reading::ReadingState;
use std::sync::Arc;

struct InlineReaderBuilder {
    width: usize,
}

impl InlineReaderBuilder {
    fn new() -> Self {
        Self { width: 40 }
    }

    fn document(&self) -> Arc<RenderedDocument> {
        Arc::new(RenderedDocument::with_layout(
            0,
            parse_document("first.xhtml", "<p>Before the image.</p><img src='same.png'/><p>Between the images.</p><img src='same.png'/><p>After the image.</p>").unwrap(),
            LayoutOptions { width: self.width, starting_row: 0, image_rows: (self.width / 4).clamp(3, 24) },
        ))
    }

    fn reader(&self) -> ReaderModel {
        let baseline = reader_support::reader();
        let mut configuration = baseline.view().configuration.clone();
        configuration.settings.between_chapters = false;
        ReaderModel::new(
            ReaderSession {
                info: baseline.view().info.clone(),
                rendered: self.document(),
                configuration,
                position: ReadingState {
                    content_id: "first.xhtml".into(),
                    textwidth: self.width,
                    ..ReadingState::default()
                },
            },
            Viewport {
                columns: 100,
                rows: 2,
            },
        )
    }
}

fn keys(reader: &mut ReaderModel, text: &str) -> Vec<ReaderEffect> {
    text.chars()
        .flat_map(|character| reader.handle_key(InputKey::Character(character)))
        .collect()
}

fn move_to(reader: &mut ReaderModel, row: usize) {
    let current = reader.view().row;
    if current == row {
        return;
    }
    let direction = if row > current { 'j' } else { 'k' };
    assert!(keys(reader, &format!("{}{direction}", current.abs_diff(row))).is_empty());
    assert_eq!(reader.view().row, row);
}

fn finish_load(reader: &mut ReaderModel, effects: Vec<ReaderEffect>) {
    let request = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("The image position must request its layout.");
    let mut builder = InlineReaderBuilder::new();
    builder.width = request.width;
    reader.accept_loaded(builder.document(), request);
}

fn request_image(reader: &mut ReaderModel) -> ImageJob {
    reader.handle_key(InputKey::Character('o'));
    reader
        .handle_key(InputKey::Enter)
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Image(job) => Some(job),
            _ => None,
        })
        .expect("Selecting an image must start loading it")
}

#[test]
fn reopening_an_image_rejects_the_previous_load_completion() {
    let mut reader = InlineReaderBuilder::new().reader();
    let image = *reader
        .view()
        .rendered
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap();
    move_to(&mut reader, image);
    let first = request_image(&mut reader);
    reader.handle_key(InputKey::Escape);
    let second = request_image(&mut reader);
    reader.image_completed(ImageResult {
        generation: 1,
        request_id: first.request_id,
        result: Err("Old image failure".into()),
    });
    assert!(
        matches!(reader.view().mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Loading { request_id } if *request_id == second.request_id))
    );
    reader.image_completed(ImageResult {
        generation: 2,
        request_id: second.request_id,
        result: Err("Current image failure".into()),
    });
    assert!(
        matches!(reader.view().mode, ReaderMode::Images(mode) if matches!(mode.as_ref(), ImageMode::Failed(error) if error == "Current image failure"))
    );
}

#[test]
fn quickmarks_restore_partial_inline_image_rows() {
    let mut reader = InlineReaderBuilder::new().reader();
    let row = *reader
        .view()
        .rendered
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap()
        + 4;
    move_to(&mut reader, row);
    keys(&mut reader, "m1");
    keys(&mut reader, "8j");
    let effects = keys(&mut reader, "`1");
    finish_load(&mut reader, effects);
    assert_eq!(reader.view().row, row);
}

#[test]
fn history_distinguishes_two_offsets_inside_the_same_image() {
    let mut reader = InlineReaderBuilder::new().reader();
    let image = *reader
        .view()
        .rendered
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap();
    move_to(&mut reader, image + 2);
    keys(&mut reader, "m1");
    keys(&mut reader, "4j");
    let effects = keys(&mut reader, "`1");
    finish_load(&mut reader, effects);
    let effects = reader.handle_key(InputKey::Character('\u{0f}'));
    finish_load(&mut reader, effects);
    assert_eq!(reader.view().row, image + 6);
}

#[test]
fn resizing_preserves_the_partial_image_occurrence() {
    let mut reader = InlineReaderBuilder::new().reader();
    let image = *reader
        .view()
        .rendered
        .layout
        .image_maps
        .keys()
        .nth(1)
        .unwrap();
    move_to(&mut reader, image + 9);
    let effects = reader.resize(Viewport {
        columns: 20,
        rows: 2,
    });
    finish_load(&mut reader, effects);
    assert_eq!(
        reader.view().row,
        *reader
            .view()
            .rendered
            .layout
            .image_maps
            .keys()
            .nth(1)
            .unwrap()
            + 3
    );
}

#[test]
fn returning_to_a_book_restores_its_partial_image_position() {
    let builder = InlineReaderBuilder::new();
    let mut departing = builder.reader();
    let row = *departing
        .view()
        .rendered
        .layout
        .image_maps
        .keys()
        .nth(1)
        .unwrap()
        + 4;
    move_to(&mut departing, row);
    let memory = departing.remember_session();
    let mut returning = builder.reader();
    let position = returning.restore_session(memory).unwrap();
    let effects = returning.initial_reading(position);
    finish_load(&mut returning, effects);
    assert_eq!(returning.view().row, row);
}

#[test]
fn persistent_positions_exclude_partial_inline_image_rows() {
    let mut reader = InlineReaderBuilder::new().reader();
    let image = *reader
        .view()
        .rendered
        .layout
        .image_maps
        .keys()
        .next()
        .unwrap();
    move_to(&mut reader, image);
    let expected = reader.position();
    move_to(&mut reader, image + 4);
    assert_eq!(reader.save_request(false).position, expected);
}
