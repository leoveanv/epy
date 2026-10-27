use serde::{Deserialize, Serialize};

use super::ViewPosition;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadingState {
    pub content_id: String,
    pub textwidth: usize,
    pub offset: usize,
    pub screen_row: usize,
}

impl Default for ReadingState {
    fn default() -> Self {
        Self {
            content_id: String::new(),
            textwidth: 80,
            offset: 0,
            screen_row: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub position: ReadingState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageDirection {
    Forward,
    Backward,
}

#[derive(Clone, Debug)]
pub struct LoadRequest {
    pub index: usize,
    pub width: usize,
    pub position: Option<ViewPosition>,
    pub at_end: bool,
    pub fragment: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SaveRequest {
    pub position: ReadingState,
    pub progress: Option<f64>,
    pub quitting: bool,
}

#[derive(Clone, Debug)]
pub struct BookmarkRequest {
    pub name: String,
    pub position: ReadingState,
}
