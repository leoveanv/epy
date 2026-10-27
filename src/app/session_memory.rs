use super::{ReaderModel, navigation::NavigationState};

pub struct BookSessionMemory {
    path: String,
    position: crate::reading::ViewPosition,
    navigation: NavigationState,
    spread: usize,
    color_scheme: usize,
    show_progress: bool,
    focused_annotation: Option<String>,
}

impl BookSessionMemory {
    pub fn spread(&self) -> usize {
        self.spread
    }
}

impl ReaderModel {
    pub fn remember_session(&self) -> BookSessionMemory {
        BookSessionMemory {
            path: self.info.path.clone(),
            position: self.view_position(),
            navigation: self.navigation.clone(),
            spread: self.spread,
            color_scheme: self.color_scheme,
            show_progress: self.show_progress,
            focused_annotation: self.annotation_state.focused().map(str::to_owned),
        }
    }

    pub fn restore_session(
        &mut self,
        memory: BookSessionMemory,
    ) -> Option<crate::reading::ViewPosition> {
        if memory.path != self.info.path {
            return None;
        }
        self.navigation = memory.navigation;
        self.spread = memory.spread;
        self.color_scheme = memory.color_scheme;
        self.show_progress = memory.show_progress;
        let focused = memory.focused_annotation.filter(|id| {
            self.info
                .annotations
                .iter()
                .any(|annotation| annotation.id == *id)
        });
        self.annotation_state.set_focused(focused);
        self.row = self.row.min(self.last_row());
        Some(memory.position)
    }
}
