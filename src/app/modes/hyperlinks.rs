use super::{ReaderEffect, ReaderMode, ReaderModel};
use crate::configuration::InputKey;
use crate::document::links::TextLink;
use crate::reading::DocumentRow;
use crate::reading::{HintPlacement, HyperlinkRequest, LinkHint, link_hints, place_hints};

#[derive(Clone, Debug)]
pub struct LinkSelection {
    pub hints: Vec<LinkHint>,
    pub placements: Option<Vec<HintPlacement>>,
    pub digits: String,
    pub first: usize,
}

impl ReaderModel {
    pub(in crate::app) fn begin_hyperlinks(&mut self) -> Vec<ReaderEffect> {
        let hints = link_hints(
            &self.window,
            DocumentRow {
                document_index: self.rendered.index,
                row: self.row,
            },
            self.viewport.rows * self.spread,
        );
        if hints.is_empty() {
            self.status = "No visible links.".into();
        } else {
            self.status.clear();
            let placements = place_hints(&hints, self.width);
            self.mode = ReaderMode::Hyperlinks(LinkSelection {
                hints,
                placements,
                digits: String::new(),
                first: 0,
            });
        }
        Vec::new()
    }

    pub(in crate::app) fn hyperlink_key(&mut self, key: InputKey) -> Vec<ReaderEffect> {
        let ReaderMode::Hyperlinks(mut selection) =
            std::mem::replace(&mut self.mode, ReaderMode::Reading)
        else {
            return Vec::new();
        };
        match key {
            InputKey::Escape | InputKey::Character('\u{03}') => return Vec::new(),
            InputKey::Character(digit) if digit.is_ascii_digit() && selection.digits.len() < 8 => {
                selection.digits.push(digit)
            }
            InputKey::Backspace => {
                selection.digits.pop();
            }
            InputKey::Up if selection.placements.is_none() => {
                selection.first = selection.first.saturating_sub(1)
            }
            InputKey::Down if selection.placements.is_none() => {
                selection.first = (selection.first + 1).min(selection.hints.len().saturating_sub(1))
            }
            InputKey::Enter => {
                if let Ok(number) = selection.digits.parse::<usize>()
                    && let Some(hint) = number
                        .checked_sub(1)
                        .and_then(|index| selection.hints.get(index))
                {
                    self.begin_jump();
                    return self.request_hyperlink(hint.link.clone());
                }
                selection.digits.clear();
            }
            _ => {}
        }
        self.mode = ReaderMode::Hyperlinks(selection);
        Vec::new()
    }

    pub(in crate::app) fn request_hyperlink(&mut self, link: TextLink) -> Vec<ReaderEffect> {
        self.loading = true;
        self.loading_request = None;
        self.pending_hyperlink = Some(link.clone());
        self.status = "Opening link…".into();
        vec![ReaderEffect::Hyperlink(HyperlinkRequest {
            link,
            width: self.width,
            continuous_rows: self
                .configuration
                .settings
                .between_chapters
                .then_some(self.viewport.rows * self.spread),
        })]
    }

    pub fn external_link_ready(&mut self, url: url::Url) -> Vec<ReaderEffect> {
        self.clear_pending_jump();
        self.loading = false;
        self.pending_hyperlink = None;
        self.status.clear();
        let mut effects = if self.quit_after_load {
            Vec::new()
        } else {
            vec![ReaderEffect::Browser(url)]
        };
        if self.configuration.settings.between_chapters || self.width != self.rendered.width {
            effects.extend(self.resize(self.viewport));
        } else {
            effects.extend(self.loaded_effects());
        }
        effects
    }
}
