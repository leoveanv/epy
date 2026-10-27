use ers::annotations::{Annotation, TextAnchor};

#[path = "books.rs"]
mod books;
pub use books::book;

pub fn annotation() -> Annotation {
    Annotation {
        id: "00000000-0000-4000-8000-000000000002".into(),
        book_id: book().id,
        anchor: TextAnchor::new("chapter.xhtml", 2, "café 👩‍🔬").unwrap(),
        note: "Personal note".into(),
        chapter: "Chapter one".into(),
        reference: None,
        content_order: 0,
        created_at: "2026-01-01T00:00:00+00:00".into(),
        updated_at: "2026-01-01T00:00:00+00:00".into(),
    }
}
