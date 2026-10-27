use std::sync::Arc;

use crate::annotations::Annotation;
use crate::book::{BookMetadata, TocEntry};
use crate::configuration::Configuration;
use crate::document::RenderedDocument;
use crate::reading::{Bookmark, ReadingState};

#[derive(Clone, Debug)]
pub struct BookInfo {
    pub path: String,
    pub metadata: BookMetadata,
    pub contents: Arc<[String]>,
    pub toc: Vec<TocEntry>,
    pub annotations: Vec<Annotation>,
    pub annotations_enabled: bool,
    pub bookmarks: Vec<Bookmark>,
}

pub struct ReaderSession {
    pub info: Arc<BookInfo>,
    pub rendered: Arc<RenderedDocument>,
    pub position: ReadingState,
    pub configuration: Configuration,
}
