use ers::annotations::{Annotation, AnnotationBook, ExportLink};
use ers::book::LibraryEntry;
use ers::reading::ReadingState;
use serde::Deserialize;

mod annotations;
pub use annotations::{annotation, book};

pub fn link(path: &str) -> ExportLink {
    ExportLink {
        book_id: book().id,
        path: path.into(),
        digest: None,
        revision: 0,
        exported_revision: -1,
    }
}

pub fn reading_state() -> ReadingState {
    ReadingState {
        content_id: "chapter.xhtml".into(),
        textwidth: 80,
        offset: 72,
        screen_row: 0,
    }
}

pub fn library_entry(path: &str) -> LibraryEntry {
    LibraryEntry {
        filepath: path.into(),
        last_read: "2026-01-01 12:00:00".into(),
        title: Some(book().title),
        author: Some(book().author),
        reading_progress: Some(0.125),
    }
}

#[derive(Deserialize)]
pub struct FixtureBundle {
    pub exports: Vec<ExportFixture>,
    pub documents: Vec<DocumentFixture>,
    pub wrapping: Vec<WrapFixture>,
    pub cells: Vec<CellFixture>,
}

#[derive(Deserialize)]
pub struct WrapFixture {
    pub text: String,
    pub width: usize,
    pub lines: Vec<String>,
}

#[derive(Deserialize)]
pub struct CellFixture {
    pub text: String,
    pub width: usize,
    pub graphemes: Vec<String>,
}

#[derive(Deserialize)]
pub struct DocumentFixture {
    pub name: String,
    pub html: String,
    pub paragraphs: Vec<String>,
    pub canonical: String,
}

#[derive(Deserialize)]
pub struct ExportFixture {
    pub name: String,
    pub book: AnnotationBook,
    pub annotations: Vec<Annotation>,
    pub newline: String,
    pub original: String,
    pub expected_digest: Option<String>,
    pub overwrite: bool,
    pub section: String,
    pub merged: String,
    pub digest: String,
}
