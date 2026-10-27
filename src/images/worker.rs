use super::svg::ImageDecoder;
use super::{DecodedImage, ImageTarget};
use crate::book::{Book, BookLocation, BookOpenOptions};
use crate::cancellation::Cancellation;
use crate::{Error, Result};
use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::SystemTime;

pub struct ImageJob {
    pub request_id: u64,
    pub target: ImageTarget,
}

#[derive(Debug)]
pub struct ImagePayload {
    pub name: String,
    pub bytes: Vec<u8>,
    pub decoded: std::result::Result<DecodedImage, String>,
}

impl ImagePayload {
    fn memory_bytes(&self) -> usize {
        self.bytes.len().saturating_add(
            self.decoded
                .as_ref()
                .map_or(0, |image| image.pixels.as_bytes().len()),
        )
    }
}

pub struct ImageResult {
    pub generation: u64,
    pub request_id: u64,
    pub result: std::result::Result<Arc<ImagePayload>, String>,
}

struct ImageRequest {
    location: BookLocation,
    image: ImageJob,
}

type Mailbox = crate::worker::ReplaceableMailbox<ImageRequest, ImageResult>;
type ImageEnvelope = crate::worker::WorkerRequest<ImageRequest>;

pub struct ImageWorker {
    mailbox: Arc<Mailbox>,
    _thread: JoinHandle<()>,
}

impl ImageWorker {
    pub fn start(options: super::ImageWorkerOptions) -> Self {
        let mailbox = Arc::new(Mailbox::default());
        let worker = mailbox.clone();
        let thread = std::thread::spawn(move || run(&worker, options));
        Self {
            mailbox,
            _thread: thread,
        }
    }

    pub fn request(&self, location: BookLocation, job: ImageJob) -> u64 {
        self.mailbox.submit(ImageRequest {
            location,
            image: job,
        })
    }

    pub fn cancel(&self) {
        self.mailbox.cancel();
    }

    pub fn take_result(&self) -> Option<ImageResult> {
        self.mailbox.take_result()
    }
}

// #NOTE: One read-only decoder may finish after cancellation; it must never delay terminal cleanup.
impl Drop for ImageWorker {
    fn drop(&mut self) {
        self.mailbox.stop();
    }
}

fn run(mailbox: &Mailbox, options: super::ImageWorkerOptions) {
    let mut cache = ImageCache {
        entries: VecDeque::new(),
        bytes: 0,
        decoder: ImageDecoder::new(options),
    };
    while let Some(envelope) = mailbox.receive() {
        let result = load_cached(&envelope, &mut cache).map_err(|error| error.to_string());
        mailbox.publish(
            envelope.generation,
            ImageResult {
                generation: envelope.generation,
                request_id: envelope.job.image.request_id,
                result,
            },
        );
    }
}

fn load_cached(envelope: &ImageEnvelope, cache: &mut ImageCache) -> Result<Arc<ImagePayload>> {
    envelope.cancellation.check()?;
    let key = ImageKey::new(&envelope.job.location, &envelope.job.image.target)?;
    if let Some(payload) = cache.get(&key) {
        return Ok(payload);
    }
    let payload = Arc::new(load_image(
        &envelope.job.location,
        &envelope.job.image.target,
        &envelope.cancellation,
        &mut cache.decoder,
    )?);
    if key != ImageKey::new(&envelope.job.location, &envelope.job.image.target)? {
        return Err(Error::Compatibility(
            "The ebook changed while loading its image. Reopen it and retry.".into(),
        ));
    }
    cache.insert(key, payload.clone());
    Ok(payload)
}

fn load_image(
    location: &BookLocation,
    target: &ImageTarget,
    cancellation: &Cancellation,
    decoder: &mut ImageDecoder,
) -> Result<ImagePayload> {
    let mut book = Book::open_with_options(
        location.clone(),
        BookOpenOptions {
            cancellation: cancellation.clone(),
            ..BookOpenOptions::default()
        },
    )?;
    let resource = book.resolve_resource(&target.content_id, &target.path)?;
    let asset = book.asset(&resource.content_id)?;
    cancellation.check()?;
    let decoded = match decoder.decode(&mut book, &resource.content_id, &asset.bytes, cancellation)
    {
        Err(Error::Canceled) => return Err(Error::Canceled),
        result => result.map_err(|error| error.to_string()),
    };
    Ok(ImagePayload {
        name: asset.name,
        bytes: asset.bytes,
        decoded,
    })
}

#[derive(PartialEq, Eq)]
struct FileRevision {
    length: u64,
    modified: Option<SystemTime>,
}

#[derive(PartialEq, Eq)]
struct ImageKey {
    book: String,
    content_id: String,
    path: String,
    revision: Option<FileRevision>,
}

impl ImageKey {
    fn new(location: &BookLocation, target: &ImageTarget) -> Result<Self> {
        let revision = match location {
            BookLocation::Local(path) => {
                let metadata = std::fs::metadata(path)?;
                Some(FileRevision {
                    length: metadata.len(),
                    modified: metadata.modified().ok(),
                })
            }
            BookLocation::Web(_) => None,
        };
        Ok(Self {
            book: location.storage_key()?,
            content_id: target.content_id.clone(),
            path: target.path.clone(),
            revision,
        })
    }
}

struct CachedImage {
    key: ImageKey,
    payload: Arc<ImagePayload>,
}

struct ImageCache {
    entries: VecDeque<CachedImage>,
    bytes: usize,
    decoder: ImageDecoder,
}

impl ImageCache {
    fn get(&mut self, key: &ImageKey) -> Option<Arc<ImagePayload>> {
        let index = self.entries.iter().position(|entry| entry.key == *key)?;
        let entry = self.entries.remove(index)?;
        let payload = entry.payload.clone();
        self.entries.push_back(entry);
        Some(payload)
    }

    fn insert(&mut self, key: ImageKey, payload: Arc<ImagePayload>) {
        self.bytes = self.bytes.saturating_add(payload.memory_bytes());
        self.entries.push_back(CachedImage { key, payload });
        while self.entries.len() > 6 || self.bytes > 96 * 1024 * 1024 {
            let Some(entry) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(entry.payload.memory_bytes());
        }
    }
}
