use super::DocumentMatches;
use crate::document::RenderedDocument;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDirection {
    Forward,
    Backward,
}

#[derive(Clone)]
pub struct SearchJob {
    pub request_id: u64,
    pub pattern: String,
    pub contents: Arc<[String]>,
    pub current: Arc<RenderedDocument>,
    pub start_row: usize,
    pub start_offset: Option<usize>,
    pub after_hit: Option<usize>,
    pub direction: SearchDirection,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct SearchSelection {
    pub document: Arc<RenderedDocument>,
    pub matches: DocumentMatches,
    pub selected: usize,
    pub exhausted: bool,
}

pub struct SearchResult {
    pub request_id: u64,
    pub result: std::result::Result<Option<SearchSelection>, String>,
}
