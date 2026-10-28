use ers::annotations::{Selection, SelectionMotion, SelectionWindow};
use ers::app::editor::{EditorAction, EditorOptions, EditorState};
use ers::configuration::InputKey;
use ers::document::layout::{LayoutOptions, TextLayout, layout_document};
use ers::document::text::{ParsedDocument, parse_document};

struct SelectionFixture {
    document: ParsedDocument,
    layout: TextLayout,
}

impl SelectionFixture {
    fn new(html: &str, width: usize) -> Self {
        let document = parse_document("chapter.xhtml", html).unwrap();
        let layout = layout_document(
            &document,
            LayoutOptions {
                width,
                starting_row: 0,
                image_rows: 1,
            },
        );
        Self { document, layout }
    }

    fn selection(&self) -> Selection {
        Selection::from_viewport(&self.layout, 0, self.layout.text_lines.len()).unwrap()
    }
}

#[test]
fn selection_begins_near_the_viewport_midpoint() {
    let fixture = SelectionFixture::new("<p>one</p><p>two</p><p>three</p><p>four</p>", 20);
    let selection = Selection::from_viewport(&fixture.layout, 0, 8).unwrap();
    assert_eq!(selection.cell(&fixture.layout).unwrap().row, 4);
}

#[test]
fn reversed_selection_includes_both_grapheme_endpoints() {
    let fixture = SelectionFixture::new("<p>A é👩‍🔬 Z</p>", 80);
    let mut selection = fixture.selection();
    selection.move_cursor(SelectionMotion::Right, &fixture.layout);
    selection.move_cursor(SelectionMotion::Right, &fixture.layout);
    selection.move_cursor(SelectionMotion::Right, &fixture.layout);
    selection.start_range();
    selection.move_cursor(SelectionMotion::Left, &fixture.layout);
    assert_eq!(
        selection
            .anchor(&fixture.document, &fixture.layout)
            .unwrap()
            .quote(),
        "é👩‍🔬"
    );
}

#[test]
fn word_movement_treats_punctuation_as_a_separate_run() {
    let fixture = SelectionFixture::new("<p>alpha,beta gamma</p>", 80);
    let mut selection = fixture.selection();
    selection.move_cursor(SelectionMotion::WordForward, &fixture.layout);
    assert_eq!(selection.cell(&fixture.layout).unwrap().text, ",");
    selection.move_cursor(SelectionMotion::WordForward, &fixture.layout);
    assert_eq!(selection.cell(&fixture.layout).unwrap().text, "b");
}

#[test]
fn half_page_movement_skips_nonselectable_image_rows() {
    let fixture = SelectionFixture::new("<p>before</p><img src='picture.png'/><p>after</p>", 40);
    let mut selection = Selection::from_viewport(&fixture.layout, 0, 2).unwrap();
    selection.move_rows(4, &fixture.layout);
    assert!(
        selection.cell(&fixture.layout).unwrap().start
            >= fixture.document.source.text.find("after").unwrap()
    );
}

#[test]
fn selection_scrolloff_keeps_context_around_the_cursor() {
    let fixture = SelectionFixture::new(
        "<p>one</p><p>two</p><p>three</p><p>four</p><p>five</p><p>six</p>",
        20,
    );
    let mut selection = Selection::from_viewport(&fixture.layout, 0, 4).unwrap();
    selection.move_rows(4, &fixture.layout);
    let row = selection.cell(&fixture.layout).unwrap().row;
    assert_eq!(
        selection.viewport(
            &fixture.layout,
            SelectionWindow {
                top: 0,
                height: 4,
                scrolloff: 1
            }
        ),
        row - 2
    );
}

#[test]
fn reflow_preserves_an_unsaved_selection_anchor() {
    let fixture = SelectionFixture::new("<p>first café 👩‍🔬 and second words across lines</p>", 40);
    let mut selection = fixture.selection();
    selection.start_range();
    for _ in 0..14 {
        selection.move_cursor(SelectionMotion::Right, &fixture.layout);
    }
    let before = selection
        .anchor(&fixture.document, &fixture.layout)
        .unwrap();
    let narrow = layout_document(
        &fixture.document,
        LayoutOptions {
            width: 12,
            starting_row: 0,
            image_rows: 1,
        },
    );
    selection.remap(&fixture.layout, &narrow).unwrap();
    assert_eq!(
        selection.anchor(&fixture.document, &narrow).unwrap(),
        before
    );
}

#[test]
fn backspace_deletes_a_complete_joined_emoji() {
    let mut editor = EditorState::new("prefix é👩‍🔬", EditorOptions::default());
    editor.handle(InputKey::Backspace);
    assert_eq!(editor.text(), "prefix é");
    editor.handle(InputKey::Backspace);
    assert_eq!(editor.text(), "prefix ");
}

#[test]
fn multiline_paste_preserves_unicode_and_whitespace() {
    let mut editor = EditorState::new("", EditorOptions::default());
    editor.insert("First 👩‍🔬\r\n  Second\tline\n");
    assert_eq!(editor.text(), "First 👩‍🔬\n  Second    line\n");
}

#[test]
fn enter_inserts_a_newline_until_save_has_focus() {
    let mut editor = EditorState::new("first", EditorOptions::default());
    assert_eq!(editor.handle(InputKey::Enter), None);
    editor.insert("second");
    editor.handle(InputKey::Tab);
    assert_eq!(editor.handle(InputKey::Enter), Some(EditorAction::Save));
    assert_eq!(editor.text(), "first\nsecond");
}

#[test]
fn note_delete_is_available_without_changing_the_draft() {
    let mut editor = EditorState::new(
        "Keep this highlight",
        EditorOptions {
            multiline: true,
            can_delete: true,
        },
    );
    editor.handle(InputKey::Tab);
    editor.handle(InputKey::Tab);
    assert_eq!(editor.handle(InputKey::Enter), Some(EditorAction::Delete));
    assert_eq!(editor.text(), "Keep this highlight");
}

#[test]
fn editor_layout_keeps_wide_graphemes_intact() {
    let editor = EditorState::new("ab👩‍🔬c", EditorOptions::default());
    assert_eq!(editor.layout(3).lines, ["ab", "👩‍🔬c", ""]);
}

#[test]
fn editor_moves_fitting_words_to_the_next_line() {
    let editor = EditorState::new("first know", EditorOptions::default());
    let layout = editor.layout(8);
    assert_eq!(layout.lines, ["first ", "know"]);
    assert_eq!((layout.cursor_row, layout.cursor_col), (1, 4));
}

#[test]
fn editor_keeps_the_cursor_inside_a_wrapped_word() {
    let mut editor = EditorState::new("first know", EditorOptions::default());
    editor.handle(InputKey::Left);
    editor.handle(InputKey::Left);
    let layout = editor.layout(8);
    assert_eq!((layout.cursor_row, layout.cursor_col), (1, 2));
}

#[test]
fn editor_wraps_after_a_full_line_without_a_blank_space_line() {
    let editor = EditorState::new("hello world", EditorOptions::default());
    assert_eq!(editor.layout(5).lines, ["hello", "world", ""]);
    assert_eq!(editor.text(), "hello world");
}

#[test]
fn editor_positions_the_cursor_at_the_start_of_a_soft_wrapped_word() {
    let mut editor = EditorState::new("hello  world", EditorOptions::default());
    for _ in 0..5 {
        editor.handle(InputKey::Left);
    }
    let layout = editor.layout(5);
    assert_eq!((layout.cursor_row, layout.cursor_col), (1, 0));
}

#[test]
fn editor_keeps_unicode_words_intact_when_wrapping() {
    let editor = EditorState::new("go café", EditorOptions::default());
    assert_eq!(editor.layout(6).lines, ["go ", "café"]);
}

#[test]
fn editor_inserts_at_the_cursor_inside_a_wrapped_word() {
    let mut editor = EditorState::new("first know", EditorOptions::default());
    editor.handle(InputKey::Left);
    editor.handle(InputKey::Left);
    editor.insert("!");
    let layout = editor.layout(8);
    assert_eq!(layout.lines, ["first ", "kn!ow"]);
    assert_eq!((layout.cursor_row, layout.cursor_col), (1, 3));
    assert_eq!(editor.text(), "first kn!ow");
}

#[test]
fn editor_keeps_an_explicit_newline_after_soft_wrapped_whitespace() {
    let editor = EditorState::new("hello  \nnext", EditorOptions::default());
    assert_eq!(editor.layout(5).lines, ["hello", "next"]);
    assert_eq!(editor.text(), "hello  \nnext");
}

#[test]
fn editor_preserves_newlines_and_indentation_when_wrapping() {
    let editor = EditorState::new("head\n  zero know", EditorOptions::default());
    assert_eq!(editor.layout(8).lines, ["head", "  zero ", "know"]);
    assert_eq!(editor.text(), "head\n  zero know");
}

#[test]
fn editor_splits_oversized_words_only_at_grapheme_boundaries() {
    let editor = EditorState::new("a 👩‍🔬👩‍🔬👩‍🔬", EditorOptions::default());
    assert_eq!(editor.layout(4).lines, ["a ", "👩‍🔬👩‍🔬", "👩‍🔬"]);
}

#[test]
fn vertical_editor_motion_tracks_terminal_columns() {
    let mut editor = EditorState::new("中文abc\nx", EditorOptions::default());
    editor.handle(InputKey::Up);
    editor.insert("!");
    assert_eq!(editor.text(), "!中文abc\nx");
}
