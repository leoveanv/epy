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
        let mut builder = EditorLayoutBuilder::new(width, self.cursor_byte, self.options.multiline);
        let mut clusters = self.text.grapheme_indices(true).peekable();
        while let Some((start, cluster)) = clusters.next() {
            builder.push(start, cluster, clusters.clone());
        }
        builder.finish(self.text.len())
    }
}

struct EditorLayoutBuilder {
    layout: EditorLayout,
    width: usize,
    cursor_byte: usize,
    column: usize,
    multiline: bool,
    word_start: bool,
    skip_wrap_whitespace: bool,
}

impl EditorLayoutBuilder {
    fn new(width: usize, cursor_byte: usize, multiline: bool) -> Self {
        Self {
            layout: EditorLayout {
                lines: vec![String::new()],
                cursor_row: 0,
                cursor_col: 0,
            },
            width: width.max(1),
            cursor_byte,
            column: 0,
            multiline,
            word_start: true,
            skip_wrap_whitespace: false,
        }
    }

    fn push<'a>(
        &mut self,
        start: usize,
        cluster: &str,
        remaining: impl Iterator<Item = (usize, &'a str)>,
    ) {
        if self.consume_wrap_whitespace(start, cluster) {
            return;
        }
        let newline = matches!(cluster, "\n" | "\r\n");
        let whitespace = cluster.chars().all(char::is_whitespace);
        let size = cell_width(cluster);
        if self.multiline
            && self.word_start
            && !whitespace
            && self.column > 0
            && word_width(cluster, remaining) > self.width.saturating_sub(self.column)
        {
            self.next_line();
        }
        if !newline && self.column > 0 && self.column + size > self.width {
            self.next_line();
        }
        self.record_cursor(start);
        if newline {
            self.next_line();
        } else {
            self.layout
                .lines
                .last_mut()
                .expect("Editor layouts always contain a line")
                .push_str(cluster);
            self.column += size;
        }
        self.word_start = whitespace;
    }

    fn consume_wrap_whitespace(&mut self, start: usize, cluster: &str) -> bool {
        let newline = matches!(cluster, "\n" | "\r\n");
        let whitespace = cluster.chars().all(char::is_whitespace);
        if self.multiline
            && whitespace
            && !newline
            && (self.skip_wrap_whitespace
                || (self.column > 0 && self.column + cell_width(cluster) > self.width))
        {
            if !self.skip_wrap_whitespace {
                self.next_line();
                self.skip_wrap_whitespace = true;
            }
            self.record_cursor(start);
            self.word_start = true;
            return true;
        }
        if self.skip_wrap_whitespace && newline {
            self.record_cursor(start);
            self.skip_wrap_whitespace = false;
            self.word_start = true;
            return true;
        }
        self.skip_wrap_whitespace = false;
        false
    }

    fn next_line(&mut self) {
        self.layout.lines.push(String::new());
        self.column = 0;
    }

    fn record_cursor(&mut self, start: usize) {
        if start == self.cursor_byte {
            self.layout.cursor_row = self.layout.lines.len() - 1;
            self.layout.cursor_col = self.column;
        }
    }

    fn finish(mut self, text_length: usize) -> EditorLayout {
        if self.cursor_byte == text_length {
            if self.column >= self.width {
                self.next_line();
            }
            self.record_cursor(text_length);
        }
        self.layout
    }
}

fn word_width<'a>(first: &str, remaining: impl Iterator<Item = (usize, &'a str)>) -> usize {
    remaining
        .map(|(_, cluster)| cluster)
        .take_while(|cluster| !cluster.chars().all(char::is_whitespace))
        .map(cell_width)
        .fold(cell_width(first), usize::saturating_add)
}
