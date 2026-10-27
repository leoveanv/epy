use super::{QueuedInput, ReaderEffect, ReaderMode, ReaderModel};
use crate::app::modes::annotations::AnnotationMode;
use crate::app::modes::dictionary::DictionaryMode;
use crate::app::modes::images::ImageMode;
use crate::app::modes::library::LibraryPhase;
use crate::app::modes::markdown_export::LinkMode;
use crate::app::modes::reassociation::ReassociationMode;
use crate::app::modes::search::SearchMode;
use crate::configuration::{Command, InputKey};
use std::mem::{Discriminant, discriminant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseContext {
    Reading,
    Dialog,
    BookmarkPrompt,
    Hyperlinks,
    Images(Discriminant<ImageMode>),
    Quickmark(bool),
    Annotations(Discriminant<AnnotationMode>),
    Link(Discriminant<LinkMode>),
    Reassociation(Discriminant<ReassociationMode>),
    Search(Discriminant<SearchMode>),
    Library(Discriminant<LibraryPhase>),
    Dictionary(Discriminant<DictionaryMode>),
}

impl From<&ReaderMode> for MouseContext {
    fn from(mode: &ReaderMode) -> Self {
        match mode {
            ReaderMode::Reading => Self::Reading,
            ReaderMode::Dialog(_) => Self::Dialog,
            ReaderMode::BookmarkPrompt(_) => Self::BookmarkPrompt,
            ReaderMode::Hyperlinks(_) => Self::Hyperlinks,
            ReaderMode::Images(mode) => Self::Images(discriminant(mode.as_ref())),
            ReaderMode::Quickmark { saving } => Self::Quickmark(*saving),
            ReaderMode::Annotations(mode) => Self::Annotations(discriminant(mode)),
            ReaderMode::Link(mode) => Self::Link(discriminant(mode.as_ref())),
            ReaderMode::Reassociation(mode) => Self::Reassociation(discriminant(mode.as_ref())),
            ReaderMode::Search(mode) => Self::Search(discriminant(mode.as_ref())),
            ReaderMode::Library(mode) => Self::Library(discriminant(&mode.phase)),
            ReaderMode::Dictionary(mode) => Self::Dictionary(discriminant(mode.as_ref())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MouseClick {
    pub column: u16,
    pub row: u16,
    pub screen_columns: u16,
    pub screen_rows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MouseAction {
    LeftClick(MouseClick),
    RightClick,
    ScrollUp,
    ScrollDown,
    Enlarge,
    Shrink,
}

pub(crate) fn first_visible_item(first: usize, selected: usize, visible_rows: usize) -> usize {
    let first = first.min(selected);
    if visible_rows == 0 {
        0
    } else if selected.saturating_sub(first) >= visible_rows {
        selected.saturating_add(1).saturating_sub(visible_rows)
    } else {
        first
    }
}

#[derive(Clone, Copy)]
enum ListSurface {
    Dialog,
    Library,
    Popup,
}

struct ListLayout {
    selected: usize,
    first: usize,
    item_count: usize,
    surface: ListSurface,
}

struct ListHit {
    index: usize,
    first: usize,
}

struct ListItems {
    count: usize,
    surface: ListSurface,
}

fn clicked_list_item(click: MouseClick, layout: ListLayout) -> Option<ListHit> {
    let columns = usize::from(click.screen_columns);
    let rows = usize::from(click.screen_rows);
    let column = usize::from(click.column);
    let row = usize::from(click.row);
    let (margin, excluded_rows) = match layout.surface {
        ListSurface::Dialog => (3, 5),
        ListSurface::Library => (3, 6),
        ListSurface::Popup => (2, 4),
    };
    let visible_rows = rows.saturating_sub(excluded_rows);
    if columns <= margin * 2
        || visible_rows == 0
        || !(margin..columns - margin).contains(&column)
        || !(2..2 + visible_rows).contains(&row)
    {
        return None;
    }
    let first = first_visible_item(layout.first, layout.selected, visible_rows);
    let index = first + row - 2;
    (index < layout.item_count).then_some(ListHit { index, first })
}

fn select_list_item(
    click: MouseClick,
    selected: &mut usize,
    first: &mut usize,
    items: ListItems,
) -> bool {
    let Some(hit) = clicked_list_item(
        click,
        ListLayout {
            selected: *selected,
            first: *first,
            item_count: items.count,
            surface: items.surface,
        },
    ) else {
        return false;
    };
    *first = hit.first;
    if hit.index == *selected {
        true
    } else {
        *selected = hit.index;
        false
    }
}

impl ReaderModel {
    pub(crate) fn handle_mouse(&mut self, action: MouseAction) -> Vec<ReaderEffect> {
        if !self.configuration.settings.mouse_support || self.quitting {
            return Vec::new();
        }
        self.input_warning = false;
        self.cancel_page_turn();
        if self.loading {
            self.queue_mouse(action);
            return Vec::new();
        }
        match &self.mode {
            ReaderMode::Reading => self.reading_mouse(action),
            ReaderMode::Dialog(_) => self.dialog_mouse(action),
            ReaderMode::Library(_) => self.library_mouse(action),
            ReaderMode::Images(_) | ReaderMode::Hyperlinks(_) => self.viewer_mouse(action),
            ReaderMode::Reassociation(mode)
                if matches!(mode.as_ref(), ReassociationMode::Sources(_)) =>
            {
                self.reassociation_mouse(action)
            }
            ReaderMode::Dictionary(mode) if matches!(mode.as_ref(), DictionaryMode::Viewing(_)) => {
                self.viewer_mouse(action)
            }
            ReaderMode::Annotations(AnnotationMode::List { .. } | AnnotationMode::Choosing(_)) => {
                self.annotation_list_mouse(action)
            }
            ReaderMode::Annotations(AnnotationMode::Selecting(_) | AnnotationMode::Viewing(_)) => {
                self.viewer_mouse(action)
            }
            _ => Vec::new(),
        }
    }

    fn queue_mouse(&mut self, action: MouseAction) {
        self.queue_input(QueuedInput::Mouse {
            action,
            context: MouseContext::from(&self.mode),
            repetitions: 1,
        });
    }

    fn reading_mouse(&mut self, action: MouseAction) -> Vec<ReaderEffect> {
        let command = match action {
            MouseAction::LeftClick(click) if click.column < click.screen_columns / 2 => {
                Command::PageUp
            }
            MouseAction::LeftClick(_) => Command::PageDown,
            MouseAction::RightClick => Command::TableOfContents,
            MouseAction::ScrollUp => Command::ScrollUp,
            MouseAction::ScrollDown => Command::ScrollDown,
            MouseAction::Enlarge => Command::Enlarge,
            MouseAction::Shrink => Command::Shrink,
        };
        let count = self.take_count();
        self.execute(command, count)
    }

    fn dialog_mouse(&mut self, action: MouseAction) -> Vec<ReaderEffect> {
        let ReaderMode::Dialog(dialog) = &mut self.mode else {
            return Vec::new();
        };
        if let MouseAction::LeftClick(click) = action {
            return if select_list_item(
                click,
                &mut dialog.selected,
                &mut self.list_top,
                ListItems {
                    count: dialog.items.len(),
                    surface: ListSurface::Dialog,
                },
            ) {
                self.handle_key(InputKey::Enter)
            } else {
                Vec::new()
            };
        }
        self.viewer_mouse(action)
    }

    fn library_mouse(&mut self, action: MouseAction) -> Vec<ReaderEffect> {
        let ReaderMode::Library(mode) = &mut self.mode else {
            return Vec::new();
        };
        match &mode.phase {
            LibraryPhase::Browsing => {}
            LibraryPhase::Waiting { .. }
                if matches!(
                    action,
                    MouseAction::ScrollUp | MouseAction::ScrollDown | MouseAction::RightClick
                ) =>
            {
                return self.viewer_mouse(action);
            }
            LibraryPhase::Opening(_) if matches!(action, MouseAction::RightClick) => {
                return self.viewer_mouse(action);
            }
            _ => return Vec::new(),
        }
        if let MouseAction::LeftClick(click) = action {
            return if select_list_item(
                click,
                &mut mode.selected,
                &mut self.list_top,
                ListItems {
                    count: mode.entries.len(),
                    surface: ListSurface::Library,
                },
            ) {
                self.handle_key(InputKey::Enter)
            } else {
                Vec::new()
            };
        }
        self.viewer_mouse(action)
    }

    fn annotation_list_mouse(&mut self, action: MouseAction) -> Vec<ReaderEffect> {
        let MouseAction::LeftClick(click) = action else {
            return self.viewer_mouse(action);
        };
        let follow = match &mut self.mode {
            ReaderMode::Annotations(AnnotationMode::List { selected }) => select_list_item(
                click,
                selected,
                &mut self.list_top,
                ListItems {
                    count: self.info.annotations.len(),
                    surface: ListSurface::Popup,
                },
            ),
            ReaderMode::Annotations(AnnotationMode::Choosing(chooser)) => select_list_item(
                click,
                &mut chooser.selected,
                &mut self.list_top,
                ListItems {
                    count: chooser.records.len(),
                    surface: ListSurface::Popup,
                },
            ),
            _ => false,
        };
        if follow {
            self.handle_key(InputKey::Enter)
        } else {
            Vec::new()
        }
    }

    fn reassociation_mouse(&mut self, action: MouseAction) -> Vec<ReaderEffect> {
        let MouseAction::LeftClick(click) = action else {
            return self.viewer_mouse(action);
        };
        let follow = match &mut self.mode {
            ReaderMode::Reassociation(mode) => match mode.as_mut() {
                ReassociationMode::Sources(picker) => select_list_item(
                    click,
                    &mut picker.selected,
                    &mut self.list_top,
                    ListItems {
                        count: picker.sources.len(),
                        surface: ListSurface::Popup,
                    },
                ),
                _ => false,
            },
            _ => false,
        };
        if follow {
            self.handle_key(InputKey::Enter)
        } else {
            Vec::new()
        }
    }

    fn viewer_mouse(&mut self, action: MouseAction) -> Vec<ReaderEffect> {
        let key = match action {
            MouseAction::RightClick => InputKey::Escape,
            MouseAction::ScrollUp => InputKey::Up,
            MouseAction::ScrollDown => InputKey::Down,
            _ => return Vec::new(),
        };
        self.handle_key(key)
    }
}
