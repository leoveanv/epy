use std::ops::Index;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImageSource {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) original_row: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImageSlot {
    source: ImageSource,
    display_row: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageSlotPosition {
    pub ordinal: usize,
    pub row_offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageLayout {
    height: usize,
    slots: Vec<ImageSlot>,
}

impl ImageLayout {
    pub(super) fn new(sources: impl IntoIterator<Item = ImageSource>, height: usize) -> Self {
        let height = height.clamp(1, 24);
        let slots = sources
            .into_iter()
            .enumerate()
            .map(|(ordinal, source)| ImageSlot {
                display_row: source.original_row + ordinal * (height - 1),
                source,
            })
            .collect();
        Self { height, slots }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn display_row(&self, original_row: usize) -> usize {
        let preceding = self
            .slots
            .partition_point(|slot| slot.source.original_row < original_row);
        original_row + preceding * (self.height - 1)
    }

    pub fn original_row(&self, display_row: usize) -> usize {
        let preceding = self
            .slots
            .partition_point(|slot| slot.display_row <= display_row);
        if let Some(slot) = preceding
            .checked_sub(1)
            .and_then(|index| self.slots.get(index))
            && display_row < slot.display_row + self.height
        {
            return slot.source.original_row;
        }
        display_row - preceding * (self.height - 1)
    }

    pub(super) fn source_row_before(
        &self,
        offset: usize,
        next_text_start: Option<usize>,
    ) -> Option<usize> {
        let index = self.slots.partition_point(|slot| slot.source.end <= offset);
        let slot = self.slots.get(index)?;
        if next_text_start.is_some_and(|start| start <= slot.source.start) {
            return None;
        }
        Some(slot.display_row)
    }

    pub fn slot_at(&self, display_row: usize) -> Option<ImageSlotPosition> {
        let ordinal = self
            .slots
            .partition_point(|slot| slot.display_row <= display_row)
            .checked_sub(1)?;
        let row_offset = display_row - self.slots[ordinal].display_row;
        (row_offset < self.height).then_some(ImageSlotPosition {
            ordinal,
            row_offset,
        })
    }

    pub fn slot_row(&self, ordinal: usize, row_offset: usize) -> Option<usize> {
        self.slots
            .get(ordinal)
            .map(|slot| slot.display_row + row_offset.min(self.height - 1))
    }

    fn extra_rows(&self) -> usize {
        self.slots.len() * (self.height - 1)
    }

    fn storage_bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<ImageSlot>()
    }
}

// #NOTE: Inline slots share one empty string instead of allocating blank rows for every image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLines {
    source: Vec<String>,
    images: ImageLayout,
    starting_row: usize,
    blank: String,
}

impl TextLines {
    pub(super) fn new(source: Vec<String>, images: ImageLayout, starting_row: usize) -> Self {
        Self {
            source,
            images,
            starting_row,
            blank: String::new(),
        }
    }

    pub fn image_layout(&self) -> &ImageLayout {
        &self.images
    }

    pub fn len(&self) -> usize {
        self.source.len() + self.images.extra_rows()
    }

    pub fn original_len(&self) -> usize {
        self.source.len()
    }

    pub fn is_empty(&self) -> bool {
        self.source.is_empty()
    }

    pub fn get(&self, row: usize) -> Option<&String> {
        if row >= self.len() {
            return None;
        }
        let display_row = self.starting_row + row;
        if self
            .images
            .slot_at(display_row)
            .is_some_and(|slot| slot.row_offset > 0)
        {
            return Some(&self.blank);
        }
        self.source
            .get(self.images.original_row(display_row) - self.starting_row)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &String> + DoubleEndedIterator {
        (0..self.len()).map(|row| &self[row])
    }

    pub fn storage_bytes(&self) -> usize {
        self.source.capacity() * std::mem::size_of::<String>()
            + self.source.iter().map(String::capacity).sum::<usize>()
            + self.images.storage_bytes()
    }
}

impl Index<usize> for TextLines {
    type Output = String;

    fn index(&self, row: usize) -> &Self::Output {
        self.get(row)
            .expect("The displayed text row is outside this layout.")
    }
}
