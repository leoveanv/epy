mod annotation_writes;
mod commands;
mod document_cache;
mod hyperlinks;
mod library;
mod link_writes;
mod reassociation;
mod search;
mod window;
use commands::{WorkerServices, run_command};
mod book;
mod export;

use book::BookWorker;
use export::{ExportContext, ExportRequest, ExportWorker};

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};

use crate::annotations::{
    AnnotationJob, AnnotationResult, ExportState, LinkJob, LinkResult, ReassociationJob,
    ReassociationResult,
};
use crate::book::BookLocation;
use crate::cancellation::Cancellation;
use crate::reading::{Bookmark, ReadingState};
use crate::{Error, Result};

use crate::app::BookInfo;
use crate::document::RenderedDocument;
use crate::reading::{BookmarkRequest, LoadRequest, SaveRequest};

pub enum WorkerEvent {
    Opened {
        info: Arc<BookInfo>,
        rendered: Arc<RenderedDocument>,
        position: ReadingState,
    },
    Loaded {
        generation: u64,
        rendered: Arc<RenderedDocument>,
        request: LoadRequest,
    },
    WindowLoaded {
        generation: u64,
        view: crate::reading::PreparedWindow,
    },
    ExternalLink {
        generation: u64,
        url: url::Url,
    },
    Totals(Vec<usize>),
    Bookmarks(Vec<Bookmark>),
    Annotation(AnnotationResult),
    Link(LinkResult),
    Reassociation(ReassociationResult),
    Export(ExportState),
    Saved {
        quitting: bool,
        error: Option<String>,
    },
    Search(crate::search::SearchResult),
    Library(crate::app::requests::LibraryResult),
    LoadFailed {
        generation: u64,
        message: String,
    },
    Failed(String),
}

struct LoadJob {
    generation: u64,
    operation: LoadOperation,
}

enum LoadOperation {
    Document(LoadRequest),
    Window(crate::reading::WindowRequest),
    Hyperlink(crate::reading::HyperlinkRequest),
}

enum WorkCommand {
    Annotation(AnnotationJob),
    Link(LinkJob),
    Reassociation(ReassociationJob),
    Bookmark(BookmarkRequest),
    Library(crate::app::requests::LibraryJob),
}

#[derive(Default)]
struct PendingWork {
    load: Option<LoadJob>,
    save: Option<SaveRequest>,
    commands: VecDeque<WorkCommand>,
    stop: bool,
}

struct Mailbox {
    pending: Mutex<PendingWork>,
    wake: Condvar,
    generation: AtomicU64,
    stopped: AtomicBool,
    cancellation: Cancellation,
}

#[derive(Clone, Copy)]
pub struct ReaderJobOptions {
    pub width: usize,
    pub inline_images: bool,
}

pub struct ReaderJobs {
    location: BookLocation,
    mailbox: Arc<Mailbox>,
    pub events: Receiver<WorkerEvent>,
    thread: Option<JoinHandle<()>>,
    search: search::SearchWorker,
}

impl ReaderJobs {
    pub fn start(location: impl Into<BookLocation>, database: PathBuf, width: usize) -> Self {
        Self::start_with_options(
            location,
            database,
            ReaderJobOptions {
                width,
                inline_images: false,
            },
        )
    }

    pub fn start_with_options(
        location: impl Into<BookLocation>,
        database: PathBuf,
        options: ReaderJobOptions,
    ) -> Self {
        let location = location.into();
        let mailbox = Arc::new(Mailbox {
            pending: Mutex::new(PendingWork::default()),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            cancellation: Cancellation::default(),
        });
        let (sender, events) = bounded(16);
        let search = search::SearchWorker::start(location.clone(), sender.clone());
        let worker_mailbox = mailbox.clone();
        let worker_location = location.clone();
        let thread = std::thread::spawn(move || {
            if let Err(error) = run_worker(
                &worker_location,
                &database,
                options,
                &worker_mailbox,
                &sender,
            ) {
                send_event(
                    &sender,
                    &worker_mailbox,
                    WorkerEvent::Failed(error.to_string()),
                );
            }
        });
        Self {
            location,
            mailbox,
            events,
            thread: Some(thread),
            search,
        }
    }

    pub(crate) fn location(&self) -> &BookLocation {
        &self.location
    }

    pub fn search(&self, job: crate::search::SearchJob) {
        self.search.request(job);
    }
    pub fn cancel_search(&self) {
        self.search.cancel();
    }

    pub fn load(&self, request: LoadRequest) {
        self.submit_load(LoadOperation::Document(request));
    }

    pub fn prepare_window(&self, request: crate::reading::WindowRequest) {
        self.submit_load(LoadOperation::Window(request));
    }

    pub fn follow_hyperlink(&self, request: crate::reading::HyperlinkRequest) {
        self.submit_load(LoadOperation::Hyperlink(request));
    }

    fn submit_load(&self, operation: LoadOperation) {
        let generation = self.mailbox.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut pending = self
            .mailbox
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.load = Some(LoadJob {
            generation,
            operation,
        });
        self.mailbox.wake.notify_one();
    }

    pub fn save(&self, request: SaveRequest) {
        let mut pending = self
            .mailbox
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.save = Some(request);
        self.mailbox.wake.notify_one();
    }

    pub fn add_bookmark(&self, request: BookmarkRequest) -> Result<()> {
        self.submit(WorkCommand::Bookmark(request))
    }
    pub fn annotate(&self, job: AnnotationJob) -> Result<()> {
        self.submit(WorkCommand::Annotation(job))
    }
    pub fn link(&self, job: LinkJob) -> Result<()> {
        self.submit(WorkCommand::Link(job))
    }

    fn submit(&self, command: WorkCommand) -> Result<()> {
        let mut pending = self
            .mailbox
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending.commands.len() >= 128 {
            return Err(Error::Compatibility(
                "Too many writes are pending. Wait for the current operations.".into(),
            ));
        }
        // #NOTE: A durable library save supersedes older queued idle snapshots.
        if matches!(&command, WorkCommand::Library(job) if job.command.departure().is_some()) {
            pending.save = None;
        }
        pending.commands.push_back(command);
        self.mailbox.wake.notify_one();
        Ok(())
    }

    pub fn reassociate(&self, job: ReassociationJob) -> Result<()> {
        self.submit(WorkCommand::Reassociation(job))
    }

    pub fn library(&self, job: crate::app::requests::LibraryJob) -> Result<()> {
        self.submit(WorkCommand::Library(job))
    }

    pub fn stop(&self) {
        self.search.cancel();
        self.mailbox.stopped.store(true, Ordering::SeqCst);
        self.mailbox.cancellation.cancel();
        self.mailbox
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stop = true;
        self.mailbox.wake.notify_one();
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.mailbox.generation.load(Ordering::SeqCst) == generation
    }
}

impl Drop for ReaderJobs {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn send_event(sender: &Sender<WorkerEvent>, mailbox: &Mailbox, mut event: WorkerEvent) {
    while !mailbox.stopped.load(Ordering::SeqCst) {
        match sender.send_timeout(event, Duration::from_millis(20)) {
            Ok(()) => return,
            Err(crossbeam_channel::SendTimeoutError::Timeout(returned)) => event = returned,
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => return,
        }
    }
}

fn run_worker(
    location: &BookLocation,
    database: &Path,
    options: ReaderJobOptions,
    mailbox: &Arc<Mailbox>,
    sender: &Sender<WorkerEvent>,
) -> Result<()> {
    let mut worker = BookWorker::open(
        location,
        database,
        book::BookWorkerOptions {
            cancellation: mailbox.cancellation.clone(),
            inline_images: options.inline_images,
        },
    )?;
    let exports = worker.annotation_book.as_ref().map(|book| {
        ExportWorker::start(ExportContext {
            database: database.to_path_buf(),
            book: book.clone(),
            events: sender.clone(),
            mailbox: mailbox.clone(),
        })
    });
    let mut position = worker
        .store
        .reading_state(&worker.info.path)?
        .unwrap_or_default();
    let fresh = position.content_id.is_empty();
    let index = if fresh {
        position.content_id = worker.info.contents[0].clone();
        0
    } else {
        worker.info.contents.iter().position(|content| *content == position.content_id)
            .ok_or_else(|| Error::Compatibility("The saved document is not present in this ebook. The saved position has not been changed.".into()))?
    };
    let width = position.textwidth.min(options.width).max(1);
    let rendered = worker.load(&LoadRequest {
        index,
        width,
        position: Some(position.clone().into()),
        at_end: false,
        fragment: None,
    })?;
    if fresh && let Some(cell) = rendered.layout.first_cell_from_row(0) {
        position.offset = cell.start;
        position.screen_row = rendered
            .layout
            .text_lines
            .image_layout()
            .original_row(cell.row);
    }
    if position.offset > rendered.parsed.source.text.chars().count() {
        return Err(Error::Compatibility(
            "The saved position is outside its document. The saved position has not been changed."
                .into(),
        ));
    }
    send_event(
        sender,
        mailbox,
        WorkerEvent::Opened {
            info: worker.info.clone(),
            rendered,
            position,
        },
    );
    let initial_export = worker.export_state();
    let initial_request = initial_export
        .link
        .clone()
        .filter(|_| initial_export.pending());
    send_event(sender, mailbox, WorkerEvent::Export(initial_export));
    if let Some(link) = initial_request
        && let Some(exports) = &exports
    {
        exports.request(ExportRequest::checked(link));
    }
    loop {
        let mut pending = mailbox
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending.stop {
            break;
        }
        if let Some(command) = pending.commands.pop_front() {
            drop(pending);
            run_command(
                &mut worker,
                command,
                WorkerServices {
                    sender,
                    mailbox,
                    exports: exports.as_ref(),
                },
            );
        } else if let Some(save) = pending.save.take() {
            drop(pending);
            let error = worker.save(&save).err().map(|error| error.to_string());
            send_event(
                sender,
                mailbox,
                WorkerEvent::Saved {
                    quitting: save.quitting,
                    error,
                },
            );
        } else if let Some(job) = pending.load.take() {
            drop(pending);
            window::run_load(&mut worker, job, mailbox, sender);
        } else if worker.next_total < worker.info.contents.len() {
            drop(pending);
            match worker.count_next() {
                Ok(()) if worker.next_total == worker.info.contents.len() => {
                    send_event(sender, mailbox, WorkerEvent::Totals(worker.totals.clone()))
                }
                Err(error) => {
                    worker.next_total = worker.info.contents.len();
                    send_event(
                        sender,
                        mailbox,
                        WorkerEvent::Failed(format!("Progress calculation failed: {error}")),
                    );
                }
                _ => {}
            }
        } else {
            drop(
                mailbox
                    .wake
                    .wait(pending)
                    .unwrap_or_else(|error| error.into_inner()),
            );
        }
    }
    Ok(())
}
