use crate::configuration::InputKey;
use crate::document::text::cell_width;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorAction {
    Save,
    Delete,
    Cancel,
}

impl EditorAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Save => "Save",
            Self::Delete => "Delete note",
            Self::Cancel => "Cancel",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EditorOptions {
    pub multiline: bool,
    pub can_delete: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            multiline: true,
            can_delete: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorLayout {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

#[derive(Clone, Debug)]
pub struct EditorState {
    text: String,
    cursor_byte: usize,
    focus: usize,
    options: EditorOptions,
    boundaries: Vec<usize>,
}

impl EditorState {
    pub fn new(text: impl Into<String>, options: EditorOptions) -> Self {
        let text = text.into();
        let cursor_byte = text.len();
        let boundaries = std::iter::once(0)
            .chain(
                text.grapheme_indices(true)
                    .map(|(start, cluster)| start + cluster.len()),
            )
            .collect();
        Self {
            text,
            cursor_byte,
            focus: 0,
            options,
            boundaries,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn focus(&self) -> usize {
        self.focus
    }
    pub fn buttons(&self) -> &[EditorAction] {
        if self.options.can_delete {
            &[
                EditorAction::Save,
                EditorAction::Delete,
                EditorAction::Cancel,
            ]
        } else {
            &[EditorAction::Save, EditorAction::Cancel]
        }
    }

    pub fn insert(&mut self, value: &str) {
        let normalized = value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        let value = normalized
            .chars()
            .filter(|character| *character == '\n' || !character.is_control())
            .map(|character| {
                if character == '\n' && !self.options.multiline {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>();
        if !value.is_empty() {
            self.replace(self.cursor_byte, self.cursor_byte, &value);
        }
    }

    pub fn handle(&mut self, key: InputKey) -> Option<EditorAction> {
        match key {
            InputKey::Escape => return Some(EditorAction::Cancel),
            InputKey::Tab => self.focus = (self.focus + 1) % (self.buttons().len() + 1),
            InputKey::BackTab => {
                self.focus = (self.focus + self.buttons().len()) % (self.buttons().len() + 1)
            }
            InputKey::Enter if self.focus > 0 => {
                return self.buttons().get(self.focus - 1).copied();
            }
            _ if self.focus > 0 => {}
            InputKey::Enter if self.options.multiline => self.insert("\n"),
            InputKey::Enter => self.focus = 1,
            InputKey::Left => self.cursor_byte = self.previous_boundary(),
            InputKey::Right => self.cursor_byte = self.next_boundary(),
            InputKey::Home => {
                self.cursor_byte = self.text[..self.cursor_byte]
                    .rfind('\n')
                    .map_or(0, |index| index + 1)
            }
            InputKey::End => {
                self.cursor_byte += self.text[self.cursor_byte..]
                    .find('\n')
                    .unwrap_or(self.text.len() - self.cursor_byte)
            }
            InputKey::Backspace if self.cursor_byte > 0 => {
                self.replace(self.previous_boundary(), self.cursor_byte, "")
            }
            InputKey::Delete if self.cursor_byte < self.text.len() => {
                self.replace(self.cursor_byte, self.next_boundary(), "")
            }
            InputKey::Up => self.move_vertical(false),
            InputKey::Down => self.move_vertical(true),
            InputKey::Character(character) if !character.is_control() => {
                self.insert(&character.to_string())
            }
            _ => {}
        }
        None
    }

    fn previous_boundary(&self) -> usize {
        self.boundaries[self
            .boundaries
            .partition_point(|boundary| *boundary < self.cursor_byte)
            .saturating_sub(1)]
    }
    fn next_boundary(&self) -> usize {
        self.boundaries[self
            .boundaries
            .partition_point(|boundary| *boundary <= self.cursor_byte)
            .min(self.boundaries.len() - 1)]
    }

    fn replace(&mut self, start: usize, end: usize, value: &str) {
        let index = self
            .boundaries
            .partition_point(|boundary| *boundary < start)
            .saturating_sub(1);
        let anchor = self.boundaries[index];
        self.text.replace_range(start..end, value);
        self.cursor_byte = start + value.len();
        self.boundaries.truncate(index + 1);
        self.boundaries.extend(
            self.text[anchor..]
                .grapheme_indices(true)
                .map(|(start, cluster)| anchor + start + cluster.len()),
        );
    }

    fn move_vertical(&mut self, down: bool) {
        let start = self.text[..self.cursor_byte]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let column = cell_width(&self.text[start..self.cursor_byte]);
        let target = if down {
            self.text[self.cursor_byte..]
                .find('\n')
                .map(|index| self.cursor_byte + index + 1)
        } else if start > 0 {
            Some(
                self.text[..start - 1]
                    .rfind('\n')
                    .map_or(0, |index| index + 1),
            )
        } else {
            None
        };
        let Some(target) = target else {
            return;
        };
        let end = self.text[target..]
            .find('\n')
            .map_or(self.text.len(), |index| target + index);
        self.cursor_byte = target;
        let mut width = 0;
        for cluster in self.text[target..end].graphemes(true) {
            width += cell_width(cluster);
            if width > column {
                break;
            }
            self.cursor_byte += cluster.len();
        }
    }

    pub fn layout(&self, width: usize) -> EditorLayout {
        let width = width.max(1);
        let mut lines = vec![String::new()];
        let mut row = 0;
        let mut col = 0;
        let mut cursor_row = 0;
        let mut cursor_col = 0;
        for (start, cluster) in self.text.grapheme_indices(true) {
            let size = cell_width(cluster);
            let newline = matches!(cluster, "\n" | "\r\n");
            if !newline && col > 0 && col + size > width {
                lines.push(String::new());
                row += 1;
                col = 0;
            }
            if start == self.cursor_byte {
                cursor_row = row;
                cursor_col = col;
            }
            if newline {
                lines.push(String::new());
                row += 1;
                col = 0;
            } else {
                lines[row].push_str(cluster);
                col += size;
            }
        }
        if self.cursor_byte == self.text.len() {
            if col >= width {
                lines.push(String::new());
                row += 1;
                col = 0;
            }
            cursor_row = row;
            cursor_col = col;
        }
        EditorLayout {
            lines,
            cursor_row,
            cursor_col,
        }
    }
}
