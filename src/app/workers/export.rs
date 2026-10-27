use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{Mailbox, WorkerEvent, send_event};
use crate::annotations::{AnnotationBook, ExportLink, ExportState, OverwriteApproval};
use crate::export::{ExportOptions, export_book};
use crate::storage::Store;
use crate::{Error, Result};
use crossbeam_channel::Sender;

pub(super) struct ExportContext {
    pub database: PathBuf,
    pub book: AnnotationBook,
    pub events: Sender<WorkerEvent>,
    pub mailbox: Arc<Mailbox>,
}

pub(super) struct ExportRequest {
    pub link: ExportLink,
    pub overwrite: Option<OverwriteApproval>,
}

impl ExportRequest {
    pub fn checked(link: ExportLink) -> Self {
        Self {
            link,
            overwrite: None,
        }
    }
}

struct ExportTask {
    request: ExportRequest,
    canceled: Arc<AtomicBool>,
}

#[derive(Default)]
struct ExportQueue {
    pending: Option<ExportTask>,
    active: Option<Arc<AtomicBool>>,
    stopped: bool,
}
struct ExportMailbox {
    state: Mutex<ExportQueue>,
    wake: Condvar,
}
pub(super) struct ExportWorker {
    mailbox: Arc<ExportMailbox>,
    thread: Option<JoinHandle<()>>,
}

impl ExportWorker {
    pub fn start(context: ExportContext) -> Self {
        let mailbox = Arc::new(ExportMailbox {
            state: Mutex::new(ExportQueue::default()),
            wake: Condvar::new(),
        });
        let worker_mailbox = mailbox.clone();
        let thread = std::thread::spawn(move || run_exports(context, worker_mailbox));
        Self {
            mailbox,
            thread: Some(thread),
        }
    }

    pub fn request(&self, request: ExportRequest) {
        let mut state = self
            .mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(active) = &state.active {
            active.store(true, Ordering::SeqCst);
        }
        state.pending = Some(ExportTask {
            request,
            canceled: Arc::new(AtomicBool::new(false)),
        });
        self.mailbox.wake.notify_one();
    }

    pub fn cancel_and_wait(&self) -> Result<()> {
        let mut state = self
            .mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.pending = None;
        if let Some(active) = &state.active {
            active.store(true, Ordering::SeqCst);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.active.is_some() {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                return Err(Error::Compatibility("The previous export is still busy. Its link has not been changed; retry shortly.".into()));
            }
            let (next, _) = self
                .mailbox
                .wake
                .wait_timeout(state, timeout)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
        }
        Ok(())
    }
}

impl Drop for ExportWorker {
    fn drop(&mut self) {
        let mut state = self
            .mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.stopped = true;
        if let Some(active) = &state.active {
            active.store(true, Ordering::SeqCst);
        }
        self.mailbox.wake.notify_one();
        drop(state);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_exports(context: ExportContext, mailbox: Arc<ExportMailbox>) {
    loop {
        let mut state = mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return;
        }
        let Some(task) = state.pending.take() else {
            drop(
                mailbox
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner()),
            );
            continue;
        };
        state.active = Some(task.canceled.clone());
        drop(state);
        let result = perform_export(&context, &task);
        if !task.canceled.load(Ordering::SeqCst) {
            send_event(
                &context.events,
                &context.mailbox,
                WorkerEvent::Export(result),
            );
        }
        mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active = None;
        mailbox.wake.notify_all();
    }
}

fn perform_export(context: &ExportContext, task: &ExportTask) -> ExportState {
    let result = Store::open(&context.database).and_then(|mut store| {
        export_book(
            &mut store,
            &context.book,
            ExportOptions {
                canceled: Some(&task.canceled),
                expected_path: Some(&task.request.link.path),
                overwrite: task.request.overwrite.as_ref(),
                ..ExportOptions::default()
            },
        )
    });
    match result {
        Ok(Some(receipt)) => ExportState {
            link: Some(ExportLink {
                book_id: receipt.book_id,
                path: receipt.path,
                digest: Some(receipt.digest),
                revision: receipt.revision,
                exported_revision: receipt.revision,
            }),
            ..ExportState::default()
        },
        Ok(None) => ExportState::default(),
        Err(error) => ExportState {
            link: Some(task.request.link.clone()),
            error: Some(error.to_string()),
            ..ExportState::default()
        },
    }
}
