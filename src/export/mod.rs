mod filesystem;
mod markdown;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::annotations::{AnnotationBook, OverwriteApproval};
use crate::storage::Store;
use crate::{Error, Result};
use filesystem::{DestinationLock, atomic_replace};

pub use filesystem::validate_destination;
pub use markdown::{
    MergeRequest, MergedSection, literal_markdown, merge_section, render_section, section_digest,
};

pub fn file_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

pub struct ExportOptions<'a> {
    pub overwrite: Option<&'a OverwriteApproval>,
    pub expected_path: Option<&'a str>,
    pub canceled: Option<&'a AtomicBool>,
    pub lock_timeout: Duration,
}

impl Default for ExportOptions<'_> {
    fn default() -> Self {
        Self {
            overwrite: None,
            expected_path: None,
            canceled: None,
            lock_timeout: Duration::from_secs(10),
        }
    }
}

impl ExportOptions<'_> {
    fn check_cancel(&self) -> Result<()> {
        if self
            .canceled
            .is_some_and(|canceled| canceled.load(Ordering::SeqCst))
        {
            return Err(Error::Canceled);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ExportReceipt {
    pub book_id: String,
    pub path: String,
    pub digest: String,
    pub revision: i64,
}

pub fn export_book(
    store: &mut Store,
    book: &AnnotationBook,
    options: ExportOptions<'_>,
) -> Result<Option<ExportReceipt>> {
    options.check_cancel()?;
    let Some(link) = store.export_link(&book.id)? else {
        return Ok(None);
    };
    if options
        .expected_path
        .is_some_and(|expected| expected != link.path)
    {
        return Err(Error::Compatibility(
            "The Markdown link changed. Reopen the dialog and retry.".into(),
        ));
    }
    let path = validate_destination(&link.path)?;
    let _lock = DestinationLock::acquire(&path, &options)?;
    let snapshot = store.export_snapshot(&book.id)?.ok_or_else(|| {
        Error::Compatibility("The book link changed during export. Retry.".into())
    })?;
    if validate_destination(&snapshot.link.path)? != path {
        return Err(Error::Compatibility(
            "The book link changed during export. Retry.".into(),
        ));
    }
    options.check_cancel()?;
    let original = std::fs::read(&path)?;
    if let Some(approval) = options.overwrite
        && (approval.link != snapshot.link || approval.file_digest != file_digest(&original))
    {
        return Err(Error::Compatibility(
            "The file or annotations changed after overwrite approval. Review and confirm again."
                .into(),
        ));
    }
    let newline = if original.windows(2).any(|pair| pair == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let section = render_section(book, &snapshot.annotations, newline);
    let merged = merge_section(MergeRequest {
        content: &original,
        book_id: &book.id,
        section: &section,
        expected_digest: snapshot.link.digest.as_deref(),
        overwrite: options.overwrite.is_some(),
    })?;
    if merged.content != original {
        atomic_replace(&path, &original, &merged.content, &options)?;
    }
    if !store.mark_exported(&snapshot.link, &merged.digest)? {
        return Err(Error::Compatibility("The Markdown link changed before export acknowledgement. Annotations remain saved; review the destination and retry.".into()));
    }
    Ok(Some(ExportReceipt {
        book_id: book.id.clone(),
        path: snapshot.link.path,
        digest: merged.digest,
        revision: snapshot.link.revision,
    }))
}
