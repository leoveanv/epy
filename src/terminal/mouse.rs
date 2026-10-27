use crate::app::BookInfo;
use crate::app::ReaderModel;
use crate::app::mouse::{MouseAction, MouseClick, MouseContext};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Size;
use std::sync::Arc;

struct MousePress {
    event: MouseEvent,
    context: MouseContext,
    book: Arc<BookInfo>,
}

#[derive(Default)]
pub(super) struct MouseTracker {
    pressed: Option<MousePress>,
}

impl MouseTracker {
    pub(super) fn cancel(&mut self) {
        self.pressed = None;
    }

    pub(super) fn input(
        &mut self,
        event: MouseEvent,
        screen: Size,
        model: Option<&ReaderModel>,
    ) -> Option<MouseAction> {
        match event.kind {
            MouseEventKind::ScrollUp if event.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(MouseAction::Enlarge)
            }
            MouseEventKind::ScrollDown if event.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(MouseAction::Shrink)
            }
            MouseEventKind::ScrollUp => Some(MouseAction::ScrollUp),
            MouseEventKind::ScrollDown => Some(MouseAction::ScrollDown),
            MouseEventKind::Down(MouseButton::Left | MouseButton::Right) => {
                self.pressed = model.map(|model| MousePress {
                    event,
                    context: MouseContext::from(model.view().mode),
                    book: model.view().info.clone(),
                });
                None
            }
            MouseEventKind::Down(_) => {
                self.cancel();
                None
            }
            MouseEventKind::Up(button) => self.release(button, event, screen, model),
            MouseEventKind::Drag(_) | MouseEventKind::Moved => {
                self.cancel();
                None
            }
            _ => None,
        }
    }

    fn release(
        &mut self,
        button: MouseButton,
        event: MouseEvent,
        screen: Size,
        model: Option<&ReaderModel>,
    ) -> Option<MouseAction> {
        let press = self.pressed.take()?;
        let model = model?;
        if press.event.kind != MouseEventKind::Down(button)
            || press.event.column != event.column
            || press.event.row != event.row
            || !press.event.modifiers.is_empty()
            || !event.modifiers.is_empty()
            || press.context != MouseContext::from(model.view().mode)
            || !Arc::ptr_eq(&press.book, model.view().info)
        {
            return None;
        }
        match button {
            MouseButton::Left => Some(MouseAction::LeftClick(MouseClick {
                column: event.column,
                row: event.row,
                screen_columns: screen.width,
                screen_rows: screen.height,
            })),
            MouseButton::Right => Some(MouseAction::RightClick),
            MouseButton::Middle => None,
        }
    }
}
