use ers::annotations::Annotation;
use ers::app::BookInfo;
use ers::app::{ReaderModel, ReaderSession, Viewport};
use ers::book::BookMetadata;
use ers::configuration::Configuration;
use ers::document::RenderedDocument;
use ers::document::text::parse_document;
use ers::reading::ReadingState;
use std::sync::Arc;

#[derive(Default)]
pub struct ReaderBuilder {
    pub configuration: Configuration,
    pub annotations: Vec<Annotation>,
}

impl ReaderBuilder {
    pub fn build(self) -> ReaderModel {
        let html = (0..100)
            .map(|index| format!("<p>Line {index} and more text for reading.</p>"))
            .collect::<String>();
        let parsed = parse_document("first.xhtml", &html).unwrap();
        let info = BookInfo {
            path: "/synthetic/book.epub".into(),
            metadata: BookMetadata {
                title: Some("Synthetic book".into()),
                ..BookMetadata::default()
            },
            contents: vec!["first.xhtml".into(), "second.xhtml".into()].into(),
            toc: Vec::new(),
            annotations: self.annotations,
            annotations_enabled: true,
            bookmarks: Vec::new(),
        };
        ReaderModel::new(
            ReaderSession {
                info: Arc::new(info),
                rendered: Arc::new(RenderedDocument::new(0, parsed, 40)),
                position: ReadingState {
                    content_id: "first.xhtml".into(),
                    textwidth: 40,
                    offset: 0,
                    screen_row: 0,
                },
                configuration: self.configuration,
            },
            Viewport {
                columns: 100,
                rows: 20,
            },
        )
    }
}

#[allow(dead_code)]
pub fn reader() -> ReaderModel {
    ReaderBuilder::default().build()
}
