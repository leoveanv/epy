#[path = "support/reader.rs"]
mod reader_support;

use ers::app::{ReaderEffect, ReaderMode, ReaderModel};
use ers::configuration::InputKey;
use ers::document::RenderedDocument;
use std::sync::Arc;

fn keys(reader: &mut ReaderModel, text: &str) -> Vec<ReaderEffect> {
    text.chars()
        .flat_map(|character| reader.handle_key(InputKey::Character(character)))
        .collect()
}

fn finish_load(reader: &mut ReaderModel, effects: Vec<ReaderEffect>) -> bool {
    let Some(request) = effects.into_iter().find_map(|effect| match effect {
        ReaderEffect::Load(request) => Some(request),
        _ => None,
    }) else {
        return false;
    };
    let mut parsed = reader.view().rendered.parsed.clone();
    parsed.source.content_id = reader.view().info.contents[request.index].clone();
    let document = Arc::new(RenderedDocument::new(request.index, parsed, request.width));
    assert!(reader.accept_loaded(document, request).is_empty());
    true
}

fn jump_mark(reader: &mut ReaderModel, mark: char) {
    let effects = keys(reader, &format!("`{mark}"));
    assert!(
        finish_load(reader, effects),
        "A quickmark must request its canonical destination"
    );
}

#[test]
fn quickmarks_restore_the_saved_viewport() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "5jm1");
    let saved = reader.position();
    keys(&mut reader, "20j");
    jump_mark(&mut reader, '1');
    assert_eq!(reader.position(), saved);
}

#[test]
fn history_forward_restores_the_latest_scrolled_arrival() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "5jm1");
    keys(&mut reader, "20j");
    let origin = reader.position();
    jump_mark(&mut reader, '1');
    keys(&mut reader, "7j");
    let arrival = reader.position();
    let backward = reader.handle_key(InputKey::Character('\u{0f}'));
    assert!(finish_load(&mut reader, backward));
    assert_eq!(reader.position(), origin);
    let forward = reader.handle_key(InputKey::ControlI);
    assert!(finish_load(&mut reader, forward));
    assert_eq!(reader.position(), arrival);
}

#[test]
fn new_jumps_discard_the_forward_branch() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "10jm110jm210jm310j");
    jump_mark(&mut reader, '1');
    jump_mark(&mut reader, '2');
    let backward = reader.handle_key(InputKey::Character('\u{0f}'));
    assert!(finish_load(&mut reader, backward));
    jump_mark(&mut reader, '3');
    let position = reader.position();
    assert!(reader.handle_key(InputKey::ControlI).is_empty());
    assert_eq!(reader.position(), position);
}

#[test]
fn history_retains_at_most_one_hundred_positions() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "m0100jm1");
    for index in 0..120 {
        jump_mark(&mut reader, if index % 2 == 0 { '0' } else { '1' });
    }
    let mut restored = 0;
    loop {
        let effects = reader.handle_key(InputKey::Character('\u{0f}'));
        if !finish_load(&mut reader, effects) {
            break;
        }
        restored += 1;
        assert!(restored <= 100);
    }
    assert_eq!(restored, 99);
}

#[test]
fn canceled_quickmark_input_does_not_execute_pasted_commands() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "m");
    reader.handle_paste("1q");
    reader.handle_key(InputKey::Escape);
    assert!(matches!(reader.view().mode, ReaderMode::Reading));
    assert!(!reader.view().quitting);
    let effects = keys(&mut reader, "`1");
    assert!(effects.is_empty());
}

#[test]
fn resize_keeps_the_pending_cross_document_jump_destination() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "m0");
    let chapter = keys(&mut reader, "L");
    assert!(finish_load(&mut reader, chapter));
    assert_eq!(reader.view().rendered.index, 1);
    keys(&mut reader, "`0");
    let resized = reader.resize(ers::app::Viewport {
        columns: 30,
        rows: 20,
    });
    let request = resized
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("Resize must reflow the pending destination");
    assert_eq!(request.index, 0);
}

#[test]
fn quickmarks_use_the_current_width_after_reflow() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "7jm1");
    let saved = reader.position();
    let reflow = keys(&mut reader, "10+");
    assert!(finish_load(&mut reader, reflow));
    keys(&mut reader, "30j");
    let effects = keys(&mut reader, "`1");
    let request = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("A quickmark must request a load");
    assert_eq!(request.width, 50);
    assert_eq!(request.position.unwrap().text.offset, saved.offset);
}

#[test]
fn history_counts_restore_multiple_positions_in_one_load() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "10jm110jm210jm310j");
    let origin = reader.position();
    jump_mark(&mut reader, '1');
    jump_mark(&mut reader, '2');
    jump_mark(&mut reader, '3');
    let backward = keys(&mut reader, "99\u{0f}");
    assert!(finish_load(&mut reader, backward));
    assert_eq!(reader.position(), origin);
    keys(&mut reader, "99");
    let forward = reader.handle_key(InputKey::ControlI);
    assert!(finish_load(&mut reader, forward));
    assert_eq!(reader.view().row, 30);
}

fn complete_pending_load(
    reader: &mut ReaderModel,
    effects: Vec<ReaderEffect>,
) -> Vec<ReaderEffect> {
    let request = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("a pending document load");
    let mut parsed = reader.view().rendered.parsed.clone();
    parsed.source.content_id = reader.view().info.contents[request.index].clone();
    let document = Arc::new(RenderedDocument::new(request.index, parsed, request.width));
    reader.accept_loaded(document, request)
}

#[test]
fn interrupt_waits_for_the_requested_chapter_before_saving() {
    let mut reader = reader_support::reader();
    let loading = keys(&mut reader, "L");
    let target = reader.view().info.contents[1].clone();
    assert!(reader.handle_key(InputKey::Character('\u{03}')).is_empty());
    let effects = complete_pending_load(&mut reader, loading);
    assert!(
        matches!(effects.as_slice(), [ReaderEffect::Quit(request)] if request.position.content_id == target)
    );
}

#[test]
fn interrupting_the_library_waits_for_its_pending_reflow() {
    let mut reader = reader_support::reader();
    keys(&mut reader, "17jR");
    let loading = reader.resize(ers::app::Viewport {
        columns: 30,
        rows: 20,
    });
    assert!(reader.handle_key(InputKey::Character('\u{03}')).is_empty());
    let effects = complete_pending_load(&mut reader, loading);
    assert!(
        matches!(effects.as_slice(), [ReaderEffect::Quit(request)] if request.position == reader.position() && request.position.textwidth == 26)
    );
}
