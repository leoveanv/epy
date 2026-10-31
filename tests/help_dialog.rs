#[path = "support/reader.rs"]
mod reader_support;

use ers::app::ReaderMode;
use ers::configuration::InputKey;
use reader_support::reader;

#[test]
fn help_displays_readable_control_keys() {
    let mut model = reader();
    model.handle_key(InputKey::Character('?'));
    let view = model.view();
    let ReaderMode::Dialog(dialog) = view.mode else {
        panic!("Help must open");
    };
    for expected in [
        "Ctrl+U  half_screen_up",
        "Ctrl+D  half_screen_down",
        "Ctrl+O  jump_backward",
        "Ctrl+I  jump_forward",
        "Space, l, Right, PageDown  page_down",
    ] {
        assert!(
            dialog.items.iter().any(|item| item.label == expected),
            "missing {expected}"
        );
    }
}
