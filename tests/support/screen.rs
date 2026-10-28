#![allow(dead_code)]

use ers::document::text::cell_width;

use crate::pty_support::PtyReader;

#[derive(Clone)]
struct ScreenCell {
    text: String,
    underlined: bool,
    dimmed: bool,
}

impl Default for ScreenCell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            underlined: false,
            dimmed: false,
        }
    }
}

pub struct Screen {
    cells: Vec<ScreenCell>,
    columns: usize,
    rows: usize,
    column: usize,
    row: usize,
    underlined: bool,
    dimmed: bool,
}

impl Screen {
    pub fn snapshot(reader: &mut PtyReader, columns: usize, rows: usize) -> Self {
        let mut screen = Self {
            cells: vec![ScreenCell::default(); columns * rows],
            columns,
            rows,
            column: 0,
            row: 0,
            underlined: false,
            dimmed: false,
        };
        vte::Parser::new().advance(&mut screen, &reader.output_since(0));
        screen
    }

    pub fn line(&self, row: usize) -> String {
        self.cells[row * self.columns..(row + 1) * self.columns]
            .iter()
            .map(|cell| cell.text.as_str())
            .collect()
    }

    pub fn text_style(&self, text: &str) -> Option<Vec<bool>> {
        self.cells.chunks(self.columns).find_map(|cells| {
            let line = cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>();
            let start = line.find(text)?;
            let start = line[..start].chars().count();
            Some(
                cells
                    .iter()
                    .flat_map(|cell| cell.text.chars().map(|_| cell.underlined))
                    .skip(start)
                    .take(text.chars().count())
                    .collect(),
            )
        })
    }

    pub fn dimmed(&self, row: usize, column: usize) -> bool {
        self.cells[row * self.columns + column].dimmed
    }

    fn clear_line(&mut self, start: usize, end: usize) {
        let base = self.row * self.columns;
        self.cells[base + start..base + end].fill(ScreenCell::default());
    }

    fn set_attributes(&mut self, values: &[u16]) {
        let mut values = values.iter().copied().peekable();
        if values.peek().is_none() {
            self.dimmed = false;
            self.underlined = false;
        }
        while let Some(value) = values.next() {
            match value {
                0 => {
                    self.dimmed = false;
                    self.underlined = false;
                }
                2 => self.dimmed = true,
                22 => self.dimmed = false,
                4 => self.underlined = true,
                24 => self.underlined = false,
                38 | 48 | 58 => match values.next() {
                    Some(5) => {
                        values.next();
                    }
                    Some(2) => {
                        values.nth(2);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

impl vte::Perform for Screen {
    fn print(&mut self, character: char) {
        let text = character.to_string();
        let width = cell_width(&text);
        if width == 0 {
            if self.column > 0 && self.row < self.rows {
                self.cells[self.row * self.columns + self.column.min(self.columns) - 1]
                    .text
                    .push(character);
            }
        } else if self.row < self.rows && self.column < self.columns {
            self.cells[self.row * self.columns + self.column] = ScreenCell {
                text,
                underlined: self.underlined,
                dimmed: self.dimmed,
            };
            self.column += width;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\r' => self.column = 0,
            b'\n' => self.row = (self.row + 1).min(self.rows - 1),
            b'\x08' => self.column = self.column.saturating_sub(1),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _: bool, action: char) {
        if !intermediates.is_empty() {
            return;
        }
        let values = params.iter().map(|part| part[0]).collect::<Vec<_>>();
        let first = usize::from(*values.first().unwrap_or(&0));
        let amount = first.max(1);
        match action {
            'H' | 'f' => {
                self.row = amount.saturating_sub(1).min(self.rows - 1);
                self.column = usize::from(*values.get(1).unwrap_or(&1))
                    .max(1)
                    .saturating_sub(1)
                    .min(self.columns - 1);
            }
            'G' => self.column = amount.saturating_sub(1).min(self.columns - 1),
            'd' => self.row = amount.saturating_sub(1).min(self.rows - 1),
            'A' => self.row = self.row.saturating_sub(amount),
            'B' => self.row = (self.row + amount).min(self.rows - 1),
            'C' => self.column = (self.column + amount).min(self.columns - 1),
            'D' => self.column = self.column.saturating_sub(amount),
            'J' if first == 2 => self.cells.fill(ScreenCell::default()),
            'K' => match first {
                0 => self.clear_line(self.column.min(self.columns), self.columns),
                1 => self.clear_line(0, (self.column + 1).min(self.columns)),
                2 => self.clear_line(0, self.columns),
                _ => {}
            },
            'm' => self.set_attributes(&values),
            _ => {}
        }
    }
}
