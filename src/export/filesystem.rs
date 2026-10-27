use crate::filesystem::sync_parent;
use crate::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn validate_destination(value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.contains(['\0', '\n', '\r']) {
        return Err(Error::Compatibility(
            "Paste a single literal absolute Markdown file path.".into(),
        ));
    }
    let path = Path::new(value);
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        return Err(Error::Compatibility(
            "The destination must be an absolute .md file path.".into(),
        ));
    }
    let resolved = resolve_destination(path, 0)?;
    if !resolved.parent().is_some_and(Path::is_dir) || (resolved.exists() && !resolved.is_file()) {
        return Err(Error::Compatibility(
            "The destination needs an existing parent directory and must be a file.".into(),
        ));
    }
    Ok(resolved)
}

fn resolve_destination(path: &Path, depth: usize) -> Result<PathBuf> {
    if depth >= 40 {
        return Err(Error::Compatibility(
            "The destination contains too many symbolic links.".into(),
        ));
    }
    if path.is_symlink() {
        let target = std::fs::read_link(path)?;
        let target = if target.is_absolute() {
            target
        } else {
            path.parent().unwrap_or_else(|| Path::new(".")).join(target)
        };
        return resolve_destination(&target, depth + 1);
    }
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::Compatibility("The destination has no parent directory.".into()))?
        .canonicalize()?;
    let name = path
        .file_name()
        .ok_or_else(|| Error::Compatibility("The destination must be a file.".into()))?;
    Ok(parent.join(name))
}

pub(super) struct DestinationLock {
    file: File,
}

impl DestinationLock {
    pub(super) fn acquire(path: &Path, options: &super::ExportOptions<'_>) -> Result<Self> {
        let lock_path = sidecar(path, ".ers.lock")?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(false)
            .create(true)
            .open(lock_path)?;
        let started = std::time::Instant::now();
        loop {
            options.check_cancel()?;
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock)
                    if started.elapsed() < options.lock_timeout =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(std::fs::TryLockError::WouldBlock) => return Err(Error::Compatibility(
                    "The Markdown destination is locked. Retry after the other writer finishes."
                        .into(),
                )),
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        Ok(Self { file })
    }
}

impl Drop for DestinationLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn sidecar(path: &Path, suffix: &str) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| Error::Compatibility("The destination must be a file.".into()))?;
    let mut sidecar_name = std::ffi::OsString::from(".");
    sidecar_name.push(name);
    sidecar_name.push(suffix);
    Ok(path.with_file_name(sidecar_name))
}

fn require_unchanged(path: &Path, before: &[u8]) -> Result<()> {
    if std::fs::read(path)? != before {
        return Err(Error::Compatibility(
            "The destination changed during export. Retry after saving it.".into(),
        ));
    }
    Ok(())
}

fn save_backup(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Compatibility("The destination has no parent directory.".into()))?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    let metadata = std::fs::metadata(path)?;
    std::fs::copy(path, temporary.path())?;
    let mut times = std::fs::FileTimes::new();
    if let Ok(modified) = metadata.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    temporary.as_file().set_times(times)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(sidecar(path, ".ers.bak")?)
        .map_err(|error| error.error)?;
    Ok(())
}

pub(super) fn atomic_replace(
    path: &Path,
    before: &[u8],
    after: &[u8],
    options: &super::ExportOptions<'_>,
) -> Result<()> {
    options.check_cancel()?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::Compatibility("The destination has no parent directory.".into()))?;
    let permissions = std::fs::metadata(path)?.permissions();
    let mut temporary = tempfile::Builder::new()
        .prefix(".ers-export-")
        .tempfile_in(parent)?;
    temporary.write_all(after)?;
    temporary.flush()?;
    temporary.as_file().set_permissions(permissions)?;
    temporary.as_file().sync_all()?;
    require_unchanged(path, before)?;
    save_backup(path)?;
    require_unchanged(path, before)?;
    options.check_cancel()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_parent(path)
}
