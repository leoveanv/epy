use crate::images::{ImageEncoding, ImagePayload, PreparedImage, prepare_image};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::thread::JoinHandle;

pub(in crate::terminal) struct PreparationResult {
    pub version: u64,
    pub result: Result<PreparedImage, String>,
}

struct PreparationJob {
    version: u64,
    payload: Arc<ImagePayload>,
    encoding: ImageEncoding,
}

#[derive(Default)]
struct Output {
    version: AtomicU64,
    result: Mutex<Option<PreparationResult>>,
}

pub(in crate::terminal) struct ImagePreparation {
    sender: Sender<PreparationJob>,
    pending: Receiver<PreparationJob>,
    output: Arc<Output>,
    _thread: JoinHandle<()>,
}

impl ImagePreparation {
    pub fn start() -> Self {
        let (sender, pending) = bounded(1);
        let receiver = pending.clone();
        let output = Arc::new(Output::default());
        let responses = output.clone();
        let thread = std::thread::spawn(move || run(&receiver, &responses));
        Self {
            sender,
            pending,
            output,
            _thread: thread,
        }
    }

    pub fn request(&self, payload: Arc<ImagePayload>, encoding: ImageEncoding) -> u64 {
        self.cancel();
        let version = self.output.version.load(Ordering::SeqCst);
        if self
            .sender
            .try_send(PreparationJob {
                version,
                payload,
                encoding,
            })
            .is_err()
        {
            *self
                .output
                .result
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(PreparationResult {
                version,
                result: Err("The image preparation worker is unavailable.".into()),
            });
        }
        version
    }

    pub fn cancel(&self) {
        self.output.version.fetch_add(1, Ordering::SeqCst);
        self.output
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        while self.pending.try_recv().is_ok() {}
    }

    pub fn take_result(&self) -> Option<PreparationResult> {
        self.output
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

fn run(receiver: &Receiver<PreparationJob>, output: &Output) {
    while let Ok(job) = receiver.recv() {
        let result = job
            .payload
            .decoded
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|decoded| {
                prepare_image(&decoded.pixels, job.encoding).map_err(|error| error.to_string())
            });
        let mut destination = output
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if output.version.load(Ordering::SeqCst) == job.version {
            *destination = Some(PreparationResult {
                version: job.version,
                result,
            });
        }
    }
}
