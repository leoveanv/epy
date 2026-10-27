use super::document_cache::DocumentCache;
use crate::annotations::AnnotationBook;
use crate::app::BookInfo;
use crate::book::LibraryEntry;
use crate::book::{Book, BookLocation, BookOpenOptions};
use crate::cancellation::Cancellation;
use crate::document::RenderedDocument;
use crate::document::text::{parse_document, python_whitespace};
use crate::reading::{LoadRequest, SaveRequest};
use crate::storage::Store;
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

pub(super) struct BookWorkerOptions {
    pub cancellation: Cancellation,
    pub inline_images: bool,
}

pub(super) struct BookWorker {
    pub(super) book: Book,
    pub(super) annotation_book: Option<AnnotationBook>,
    pub(super) store: Store,
    pub(super) info: Arc<BookInfo>,
    cache: DocumentCache,
    pub(super) totals: Vec<usize>,
    pub(super) next_total: usize,
    inline_images: bool,
}

impl BookWorker {
    pub(super) fn open(
        location: &BookLocation,
        database: &Path,
        options: BookWorkerOptions,
    ) -> Result<Self> {
        if let Some(parent) = database.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let book = Book::open_with_options(
            location.clone(),
            BookOpenOptions {
                cancellation: options.cancellation,
                ..BookOpenOptions::default()
            },
        )?;
        let metadata = book.metadata().clone();
        let mut store = Store::open(database)?;
        let path = book.location().storage_key()?;
        let annotations_enabled = book.annotations_enabled();
        let annotation_book = if annotations_enabled {
            Some(store.identify_book(
                &AnnotationBook {
                    id: uuid::Uuid::new_v4().to_string(),
                    fingerprint: fingerprint(Path::new(&path))?,
                    title: metadata.title.clone().unwrap_or_default(),
                    author: metadata.creator.clone().unwrap_or_default(),
                },
                &path,
            )?)
        } else {
            None
        };
        let annotations = annotation_book
            .as_ref()
            .map(|book| store.annotations(&book.id))
            .transpose()?
            .unwrap_or_default();
        let info = Arc::new(BookInfo {
            contents: Arc::from(book.contents()),
            toc: book.toc().to_vec(),
            metadata,
            annotations,
            annotations_enabled,
            bookmarks: store.bookmarks(&path)?,
            path,
        });
        if info.contents.is_empty() {
            return Err(Error::Compatibility(
                "The ebook contains no readable documents.".into(),
            ));
        }
        let totals = vec![0; info.contents.len()];
        Ok(Self {
            book,
            annotation_book,
            store,
            info,
            cache: DocumentCache::new(6, 96 * 1024 * 1024),
            totals,
            next_total: 0,
            inline_images: options.inline_images,
        })
    }

    pub(super) fn annotation_book(&self) -> Result<&AnnotationBook> {
        self.annotation_book.as_ref().ok_or_else(|| {
            Error::Compatibility("Annotations are available only for local EPUB books.".into())
        })
    }

    pub(super) fn load(&mut self, request: &LoadRequest) -> Result<Arc<RenderedDocument>> {
        let options =
            crate::document::layout::LayoutOptions::reader(request.width, self.inline_images);
        let book = &mut self.book;
        let info = &self.info;
        self.cache.prepare(request.index, options, || {
            let content_id = info.contents.get(request.index).ok_or_else(|| {
                Error::Compatibility("The requested chapter is outside this book.".into())
            })?;
            parse_document(content_id, &book.raw_text(content_id)?)
        })
    }

    pub(super) fn save(&mut self, request: &SaveRequest) -> Result<()> {
        self.store.save_session(
            &LibraryEntry {
                filepath: self.info.path.clone(),
                last_read: chrono::Local::now()
                    .format("%Y-%m-%d %H:%M:%S%.6f")
                    .to_string(),
                title: self.info.metadata.title.clone(),
                author: self.info.metadata.creator.clone(),
                reading_progress: request.progress,
            },
            &request.position,
        )
    }

    pub(super) fn count_next(&mut self) -> Result<()> {
        let index = self.next_total;
        let parsed = match self.cache.parsed(index) {
            Some(parsed) => parsed.clone(),
            None => parse_document(
                &self.info.contents[index],
                &self.book.raw_text(&self.info.contents[index])?,
            )?,
        };
        self.totals[index] = parsed
            .source
            .text
            .chars()
            .filter(|character| !python_whitespace(*character))
            .count();
        self.next_total += 1;
        Ok(())
    }
}

fn fingerprint(path: &Path) -> Result<String> {
    let mut source = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = source.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
