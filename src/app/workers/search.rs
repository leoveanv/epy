use super::WorkerEvent;
use super::document_cache::DocumentCache;
use crate::book::{Book, BookLocation, BookOpenOptions};
use crate::cancellation::Cancellation;
use crate::document::RenderedDocument;
use crate::document::text::parse_document;
use crate::search::{
    SearchDirection, SearchJob, SearchLimits, SearchPattern, SearchResult, SearchSelection,
};
use crate::{Error, Result};
use crossbeam_channel::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

type Mailbox = crate::worker::ReplaceableMailbox<SearchJob, ()>;

pub(super) struct SearchWorker {
    mailbox: Arc<Mailbox>,
    thread: Option<JoinHandle<()>>,
}

impl SearchWorker {
    pub fn start(location: BookLocation, sender: Sender<WorkerEvent>) -> Self {
        let mailbox = Arc::new(Mailbox::default());
        let worker = mailbox.clone();
        let thread = std::thread::spawn(move || run(location, sender, &worker));
        Self {
            mailbox,
            thread: Some(thread),
        }
    }

    pub fn request(&self, job: SearchJob) {
        self.mailbox.submit(job);
    }

    pub fn cancel(&self) {
        self.mailbox.cancel();
    }
}

impl Drop for SearchWorker {
    fn drop(&mut self) {
        self.mailbox.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(location: BookLocation, sender: Sender<WorkerEvent>, mailbox: &Mailbox) {
    let mut cache = DocumentCache::new(3, 48 * 1024 * 1024);
    while let Some(envelope) = mailbox.receive() {
        let result = search(&location, &envelope.job, &envelope.cancellation, &mut cache)
            .map_err(|error| error.to_string());
        let mut event = WorkerEvent::Search(SearchResult {
            request_id: envelope.job.request_id,
            result,
        });
        while !envelope.cancellation.is_canceled() {
            match sender.send_timeout(event, Duration::from_millis(10)) {
                Ok(()) => break,
                Err(crossbeam_channel::SendTimeoutError::Timeout(returned)) => event = returned,
                Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => return,
            }
        }
    }
}

struct SearchTask<'a> {
    location: &'a BookLocation,
    job: &'a SearchJob,
    cancellation: &'a Cancellation,
    cache: &'a mut DocumentCache,
    book: Option<Book>,
}

impl SearchTask<'_> {
    fn load(&mut self, index: usize) -> Result<Arc<RenderedDocument>> {
        let job = self.job;
        let cancellation = self.cancellation;
        let location = self.location;
        let cache = &mut self.cache;
        let book = &mut self.book;
        cancellation.check()?;
        if index == job.current.index {
            return Ok(job.current.clone());
        }
        let options = crate::document::layout::LayoutOptions {
            width: job.current.width,
            starting_row: 0,
            image_rows: job.current.layout.text_lines.image_layout().height(),
        };
        cache.prepare(index, options, || {
            if book.is_none() {
                *book = Some(Book::open_with_options(
                    location.clone(),
                    BookOpenOptions {
                        cancellation: cancellation.clone(),
                        ..BookOpenOptions::default()
                    },
                )?);
            }
            let content_id = job.contents.get(index).ok_or_else(|| {
                Error::Compatibility("The searched document is outside this book.".into())
            })?;
            let book = book
                .as_mut()
                .ok_or_else(|| Error::Compatibility("The search book did not open.".into()))?;
            let parsed = parse_document(content_id, &book.raw_text(content_id)?)?;
            cancellation.check()?;
            Ok(parsed)
        })
    }
}

fn eligible(
    job: &SearchJob,
    index: usize,
    hit_index: usize,
    hit: &crate::search::SearchHit,
) -> bool {
    if index != job.current.index {
        return true;
    }
    match (job.direction, job.after_hit) {
        (SearchDirection::Forward, Some(previous)) => hit_index > previous,
        (SearchDirection::Backward, Some(previous)) => hit_index < previous,
        (SearchDirection::Forward, None) => {
            hit.row >= job.start_row && job.start_offset.is_none_or(|offset| hit.offset >= offset)
        }
        (SearchDirection::Backward, None) => {
            hit.row <= job.start_row && job.start_offset.is_none_or(|offset| hit.offset <= offset)
        }
    }
}

fn search(
    location: &BookLocation,
    job: &SearchJob,
    cancellation: &Cancellation,
    cache: &mut DocumentCache,
) -> Result<Option<SearchSelection>> {
    let pattern = SearchPattern::compile(&job.pattern, SearchLimits::default())?;
    let mut task = SearchTask {
        location,
        job,
        cancellation,
        cache,
        book: None,
    };
    let mut index = job.current.index;
    let mut remaining = job.count.max(1);
    let mut candidate = None;
    loop {
        cancellation.check()?;
        let document = task.load(index)?;
        let matches = pattern.find(&document.layout, cancellation)?;
        let mut selected = None;
        for ordinal in 0..matches.hits.len() {
            let hit_index = if job.direction == SearchDirection::Forward {
                ordinal
            } else {
                matches.hits.len() - 1 - ordinal
            };
            if !eligible(job, index, hit_index, &matches.hits[hit_index]) {
                continue;
            }
            selected = Some(hit_index);
            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }
        if let Some(selected) = selected {
            candidate = Some(SearchSelection {
                document,
                matches,
                selected,
                exhausted: false,
            });
        }
        if remaining == 0 {
            return Ok(candidate);
        }
        match job.direction {
            SearchDirection::Forward if index + 1 < job.contents.len() => index += 1,
            SearchDirection::Backward if index > 0 => index -= 1,
            _ => {
                if let Some(candidate) = &mut candidate {
                    candidate.exhausted = true;
                }
                return Ok(candidate);
            }
        }
    }
}
