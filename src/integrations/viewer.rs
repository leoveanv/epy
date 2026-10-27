use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Default)]
pub struct ImageViewer {
    cache: Option<ViewerCache>,
}

impl ImageViewer {
    pub fn open_rendered(&mut self, pixels: &image::DynamicImage, preference: &str) -> Result<()> {
        let mut output = std::io::Cursor::new(Vec::new());
        pixels
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|error| {
                Error::Compatibility(format!("Cannot prepare the viewer image: {error}"))
            })?;
        self.open(&output.into_inner(), preference)
    }

    pub fn open(&mut self, bytes: &[u8], preference: &str) -> Result<()> {
        let cache = match self.cache {
            Some(ref mut cache) => cache,
            None => self.cache.insert(ViewerCache::new()?),
        };
        let path = cache.prepare(bytes)?;
        if preference == "auto" {
            return open::that(&path).map_err(Error::from);
        }
        match viewer_command(preference, &path)?
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
        {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(Error::Compatibility(format!(
                "The external image viewer exited with status {status}."
            ))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                open::that(&path).map_err(Error::from)
            }
            Err(error) => Err(error.into()),
        }
    }
}

pub fn viewer_command(preference: &str, path: &Path) -> Result<Command> {
    let words = shell_words::split(preference)
        .map_err(|error| Error::Compatibility(format!("Invalid image viewer command: {error}")))?;
    let (program, arguments) = words
        .split_first()
        .filter(|(program, _)| !program.is_empty())
        .ok_or_else(|| Error::Compatibility("The image viewer command is empty.".into()))?;
    let mut command = Command::new(program);
    command.args(arguments).arg(path);
    Ok(command)
}

struct ViewerFile {
    digest: String,
    file: tempfile::NamedTempFile,
}

// #NOTE: GUI launchers return before loading files; private leases remain owned by the reader session.
struct ViewerCache {
    directory: tempfile::TempDir,
    files: Vec<ViewerFile>,
    bytes: usize,
}

impl ViewerCache {
    fn new() -> Result<Self> {
        Ok(Self {
            directory: tempfile::Builder::new().prefix("ers-images-").tempdir()?,
            files: Vec::new(),
            bytes: 0,
        })
    }

    fn prepare(&mut self, bytes: &[u8]) -> Result<PathBuf> {
        let format = image::guess_format(bytes).map_err(|_| {
            Error::Compatibility(
                "The asset is not a recognized image. It cannot open as an executable or document."
                    .into(),
            )
        })?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        if let Some(existing) = self.files.iter().find(|file| file.digest == digest) {
            return Ok(existing.file.path().to_path_buf());
        }
        if self.files.len() >= 64 || self.bytes.saturating_add(bytes.len()) > 256 * 1024 * 1024 {
            return Err(Error::Compatibility("The external image cache reached its session limit. Restart ers to release these files.".into()));
        }
        let suffix = format!(
            ".{}",
            format
                .extensions_str()
                .first()
                .ok_or_else(|| Error::Compatibility(
                    "This image has no safe external file extension.".into()
                ))?
        );
        let mut file = tempfile::Builder::new()
            .prefix("image-")
            .suffix(&suffix)
            .tempfile_in(self.directory.path())?;
        file.write_all(bytes)?;
        file.flush()?;
        let path = file.path().to_path_buf();
        self.bytes += bytes.len();
        self.files.push(ViewerFile { digest, file });
        Ok(path)
    }
}
