#[path = "support/reader.rs"]
mod reader_support;

use ers::app::{ReaderEffect, ReaderModel, Viewport};
use ers::configuration::InputKey;
use ers::document::RenderedDocument;
use ers::reading::PageDirection;
use ers::terminal::page_animation::{PageAnimation, PagePane};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::sync::Arc;
use std::time::Duration;

fn finish_load(model: &mut ReaderModel, effects: Vec<ReaderEffect>) {
    let request = effects
        .into_iter()
        .find_map(|effect| match effect {
            ReaderEffect::Load(request) => Some(request),
            _ => None,
        })
        .expect("a requested layout");
    let mut parsed = model.view().rendered.parsed.clone();
    parsed.source.content_id = model.view().info.contents[request.index].clone();
    let rendered = Arc::new(RenderedDocument::new(request.index, parsed, request.width));
    assert!(model.accept_loaded(rendered, request).is_empty());
}

fn chapter_end(model: &mut ReaderModel) {
    assert!(model.handle_key(InputKey::End).is_empty());
}

#[test]
fn page_motion_requests_the_matching_animation_direction() {
    let mut model = reader_support::reader();
    model.handle_key(InputKey::PageDown);
    assert_eq!(model.take_page_turn(), Some(PageDirection::Forward));
    model.handle_key(InputKey::PageUp);
    assert_eq!(model.take_page_turn(), Some(PageDirection::Backward));
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn chapter_boundary_animation_waits_for_successful_loading() {
    let mut model = reader_support::reader();
    chapter_end(&mut model);
    let effects = model.handle_key(InputKey::Down);
    assert_eq!(model.take_page_turn(), None);
    finish_load(&mut model, effects);
    assert_eq!(model.take_page_turn(), Some(PageDirection::Forward));
}

#[test]
fn scrolling_within_a_single_page_does_not_animate() {
    let mut model = reader_support::reader();
    model.handle_key(InputKey::Down);
    assert!(model.view().row > 0);
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn double_spread_scrolling_moves_one_full_page() {
    let mut model = reader_support::reader();
    let effects = model.handle_key(InputKey::Character('D'));
    finish_load(&mut model, effects);
    let before = model.view().row;
    model.handle_key(InputKey::Down);
    assert_eq!(
        model.view().row - before,
        model.view().viewport.rows * model.view().spread
    );
    assert_eq!(model.take_page_turn(), Some(PageDirection::Forward));
}

#[test]
fn double_spread_ignores_text_width_commands() {
    let outcomes = ["-", "22="]
        .into_iter()
        .map(|keys| {
            let mut model = reader_support::reader();
            let effects = model.handle_key(InputKey::Character('D'));
            finish_load(&mut model, effects);
            let original_width = model.view().width;
            let requested = keys
                .chars()
                .flat_map(|key| model.handle_key(InputKey::Character(key)))
                .any(|effect| matches!(effect, ReaderEffect::Load(_)));
            (model.view().width, original_width, requested)
        })
        .collect::<Vec<_>>();
    assert!(
        outcomes
            .iter()
            .all(|(width, original, requested)| { width == original && !requested })
    );
}

#[test]
fn disabled_animation_does_not_emit_page_turns() {
    let mut builder = reader_support::ReaderBuilder::default();
    builder.configuration.settings.page_scroll_animation = false;
    let mut model = builder.build();
    model.handle_key(InputKey::PageDown);
    assert!(model.view().row > 0);
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn new_input_cancels_an_animation_waiting_for_a_chapter() {
    let mut model = reader_support::reader();
    chapter_end(&mut model);
    let effects = model.handle_key(InputKey::PageDown);
    model.handle_key(InputKey::Character('z'));
    finish_load(&mut model, effects);
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn resize_cancels_an_animation_waiting_for_a_chapter() {
    let mut model = reader_support::reader();
    chapter_end(&mut model);
    let effects = model.handle_key(InputKey::PageDown);
    model.resize(Viewport {
        columns: 100,
        rows: 19,
    });
    finish_load(&mut model, effects);
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn paste_cancels_an_animation_waiting_for_a_chapter() {
    let mut model = reader_support::reader();
    chapter_end(&mut model);
    let effects = model.handle_key(InputKey::PageDown);
    model.handle_paste("ignored reader paste");
    finish_load(&mut model, effects);
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn failed_chapter_loads_cannot_start_an_animation() {
    let mut model = reader_support::reader();
    chapter_end(&mut model);
    model.handle_key(InputKey::PageDown);
    model.load_failed("Synthetic unavailable chapter".into());
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn a_page_turn_at_the_book_boundary_does_not_animate() {
    let mut model = reader_support::reader();
    model.handle_key(InputKey::PageUp);
    assert_eq!(model.take_page_turn(), None);
}

struct PageFixture {
    buffer: Buffer,
    pane: PagePane,
}

impl PageFixture {
    fn new() -> Self {
        let style = Style::default().fg(Color::White).bg(Color::Blue);
        let pane = PagePane {
            area: Rect::new(1, 0, 8, 2),
            style,
        };
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 3));
        buffer.set_stringn(0, 2, "status", 10, Style::default());
        buffer.set_style(pane.area, style);
        buffer.set_stringn(1, 0, "ABCDEFGH", 8, style);
        buffer.set_stringn(1, 1, "12345678", 8, style);
        Self { buffer, pane }
    }

    fn symbols(&self, row: u16) -> Vec<String> {
        (self.pane.area.left()..self.pane.area.right())
            .map(|column| self.buffer.cell((column, row)).unwrap().symbol().to_owned())
            .collect()
    }

    fn paint(&mut self, direction: PageDirection, elapsed_ms: u64) {
        PageAnimation::new(direction, self.pane.area.width).paint(
            &mut self.buffer,
            std::slice::from_ref(&self.pane),
            Duration::from_millis(elapsed_ms),
        );
    }
}

#[test]
fn forward_animation_slides_the_page_prefix_from_the_right() {
    let mut fixture = PageFixture::new();
    fixture.paint(PageDirection::Forward, 4);
    assert_eq!(fixture.symbols(0).concat(), "    ABCD");
}

#[test]
fn backward_animation_slides_the_page_suffix_from_the_left() {
    let mut fixture = PageFixture::new();
    fixture.paint(PageDirection::Backward, 4);
    assert_eq!(fixture.symbols(0).concat(), "EFGH    ");
}

#[test]
fn completed_animation_preserves_the_exact_target_frame() {
    let mut fixture = PageFixture::new();
    let target = fixture.buffer.clone();
    fixture.paint(PageDirection::Forward, 100);
    assert_eq!(fixture.buffer, target);
}

#[test]
fn animation_preserves_status_outside_the_reading_pane() {
    let mut fixture = PageFixture::new();
    let status = fixture.buffer.content()[20..].to_vec();
    fixture.paint(PageDirection::Forward, 0);
    assert_eq!(&fixture.buffer.content()[20..], status);
    assert_eq!(fixture.symbols(0).concat(), "        ");
}

#[test]
fn partial_animation_keeps_wide_graphemes_whole() {
    let mut fixture = PageFixture::new();
    fixture
        .buffer
        .set_stringn(1, 0, "AB👩‍🔬CDEF", 8, fixture.pane.style);
    fixture.paint(PageDirection::Forward, 3);
    assert_eq!(fixture.symbols(0).concat(), "     AB ");
}

#[test]
fn animation_keeps_the_styles_of_moving_cells() {
    let mut fixture = PageFixture::new();
    let style = Style::default()
        .fg(Color::Yellow)
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD);
    fixture.buffer.cell_mut((2, 0)).unwrap().set_style(style);
    let source = fixture.buffer.cell((2, 0)).unwrap().clone();
    fixture.paint(PageDirection::Forward, 4);
    assert_eq!(fixture.buffer.cell((6, 0)).unwrap(), &source);
}

#[test]
fn wide_page_animations_complete_within_the_input_settling_budget() {
    let animation = PageAnimation::new(PageDirection::Forward, 500);
    assert!(animation.is_finished(Duration::from_millis(80)));
}

#[test]
fn consecutive_page_turns_keep_only_the_latest_animation() {
    let mut model = reader_support::reader();
    model.handle_key(InputKey::PageDown);
    model.handle_key(InputKey::PageDown);
    model.handle_key(InputKey::PageUp);
    assert_eq!(model.take_page_turn(), Some(PageDirection::Backward));
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn unbound_input_cancels_a_ready_page_turn() {
    let mut model = reader_support::reader();
    model.handle_key(InputKey::PageDown);
    model.handle_key(InputKey::Character('z'));
    assert_eq!(model.take_page_turn(), None);
}

#[test]
fn backward_animation_keeps_suffix_graphemes_whole() {
    let mut fixture = PageFixture::new();
    fixture
        .buffer
        .set_stringn(1, 0, "A👩‍🔬BCDEF", 8, fixture.pane.style);
    fixture.paint(PageDirection::Backward, 6);
    assert_eq!(fixture.symbols(0).concat(), " BCDEF  ");
    assert_eq!(
        fixture.buffer.cell((1, 0)).unwrap().style().bg,
        fixture.pane.style.bg
    );
}

#[test]
fn animation_clips_to_the_available_frame() {
    let mut fixture = PageFixture::new();
    fixture.pane.area = Rect::new(7, 1, 8, 3);
    fixture.paint(PageDirection::Forward, 4);
    assert_eq!(fixture.buffer.cell((9, 1)).unwrap().symbol(), "7");
    assert_eq!(fixture.buffer.cell((1, 0)).unwrap().symbol(), "A");
}

#[test]
fn graphics_overlays_remain_unchanged_during_animation() {
    let mut fixture = PageFixture::new();
    fixture
        .buffer
        .cell_mut((2, 0))
        .unwrap()
        .set_diff_option(ratatui::buffer::CellDiffOption::Skip);
    let target = fixture.buffer.clone();
    fixture.paint(PageDirection::Forward, 4);
    assert_eq!(fixture.buffer, target);
}
