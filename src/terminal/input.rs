use crate::configuration::InputKey;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(super) fn input_key(event: KeyEvent) -> Option<InputKey> {
    match event.code {
        KeyCode::Char(character) if event.modifiers.contains(KeyModifiers::CONTROL) => {
            if character.eq_ignore_ascii_case(&'i') {
                Some(InputKey::ControlI)
            } else if character.is_ascii_alphabetic() {
                Some(InputKey::Character(
                    (character.to_ascii_lowercase() as u8 - b'a' + 1) as char,
                ))
            } else {
                Some(InputKey::Character(character))
            }
        }
        KeyCode::Char(character) => Some(InputKey::from_character(character)),
        KeyCode::Up => Some(InputKey::Up),
        KeyCode::Down => Some(InputKey::Down),
        KeyCode::Left => Some(InputKey::Left),
        KeyCode::Right => Some(InputKey::Right),
        KeyCode::PageUp => Some(InputKey::PageUp),
        KeyCode::PageDown => Some(InputKey::PageDown),
        KeyCode::Home => Some(InputKey::Home),
        KeyCode::End => Some(InputKey::End),
        KeyCode::Tab => Some(InputKey::Tab),
        KeyCode::Enter => Some(InputKey::Enter),
        KeyCode::Esc => Some(InputKey::Escape),
        KeyCode::Backspace => Some(InputKey::Backspace),
        KeyCode::Delete => Some(InputKey::Delete),
        KeyCode::BackTab => Some(InputKey::BackTab),
        _ => None,
    }
}
