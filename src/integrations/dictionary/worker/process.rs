use crate::integrations::{DictionaryCommand, DictionaryOptions};
use crate::{Error, Result};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt};

struct OwnedCommand {
    child: Box<dyn ChildWrapper>,
    armed: bool,
}

impl OwnedCommand {
    fn spawn(command: &DictionaryCommand, options: &DictionaryOptions) -> Result<Self> {
        let mut command = CommandWrap::with_new(&command.program, |child| {
            child
                .args(&command.arguments)
                .current_dir(&options.executables.directory)
                .env("PATH", &options.executables.path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        });
        command.wrap(KillOnDrop);
        #[cfg(unix)]
        command.wrap(process_wrap::tokio::ProcessSession);
        #[cfg(windows)]
        command.wrap(process_wrap::tokio::JobObject);
        Ok(Self {
            child: command.spawn()?,
            armed: true,
        })
    }
}

// #NOTE: process-wrap contains commands in POSIX sessions or Windows jobs; Tokio kill-on-drop alone kills only the root on Unix.
impl Drop for OwnedCommand {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.child.start_kill();
        }
    }
}

struct CapturedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: std::process::ExitStatus,
}

pub(super) async fn definition(
    command: &DictionaryCommand,
    options: &DictionaryOptions,
) -> Result<String> {
    let deadline = tokio::time::Instant::now()
        .checked_add(options.limits.timeout)
        .ok_or_else(|| Error::Compatibility("The dictionary deadline is out of range.".into()))?;
    let mut child = OwnedCommand::spawn(command, options)?;
    let stdout = child
        .child
        .stdout()
        .take()
        .ok_or_else(|| Error::Compatibility("The dictionary has no output pipe.".into()))?;
    let stderr = child
        .child
        .stderr()
        .take()
        .ok_or_else(|| Error::Compatibility("The dictionary has no error pipe.".into()))?;
    let output = tokio::time::timeout_at(
        deadline,
        capture(&mut child, stdout, stderr, options.limits.output_bytes_max),
    )
    .await
    .map_err(|_| Error::Compatibility("The dictionary deadline expired.".into()))??;
    let error = text(&output.stderr);
    if !error.trim().is_empty() {
        return Err(Error::Compatibility(error));
    }
    let definition = text(&output.stdout);
    if !output.status.success() {
        return Err(Error::Compatibility(if definition.trim().is_empty() {
            format!("The dictionary exited with {}.", output.status)
        } else {
            definition
        }));
    }
    Ok(definition)
}

// #NOTE: Drain pipes before reaping the root so cancellation retains its PID while descendant-owned pipes remain open.
async fn capture(
    child: &mut OwnedCommand,
    stdout: impl AsyncRead + Unpin,
    stderr: impl AsyncRead + Unpin,
    maximum: usize,
) -> Result<CapturedOutput> {
    let (stdout, stderr) =
        tokio::try_join!(read_limited(stdout, maximum), read_limited(stderr, maximum))?;
    let status = child.child.wait().await?;
    child.armed = false;
    Ok(CapturedOutput {
        stdout,
        stderr,
        status,
    })
}

async fn read_limited(output: impl AsyncRead + Unpin, maximum: usize) -> Result<Vec<u8>> {
    let limit = maximum.checked_add(1).ok_or_else(|| {
        Error::Compatibility("The dictionary output limit is out of range.".into())
    })?;
    let mut bytes = Vec::new();
    output.take(limit as u64).read_to_end(&mut bytes).await?;
    if bytes.len() > maximum {
        return Err(Error::Compatibility(
            "The dictionary output limit was exceeded.".into(),
        ));
    }
    Ok(bytes)
}

#[derive(Default)]
struct PlainText {
    text: String,
}

impl vte::Perform for PlainText {
    fn print(&mut self, character: char) {
        if !character.is_control() {
            self.text.push(character);
        }
    }

    fn execute(&mut self, byte: u8) {
        if matches!(byte, b'\n' | b'\t') {
            self.text.push(char::from(byte));
        }
    }
}

fn text(bytes: &[u8]) -> String {
    let mut output = PlainText::default();
    vte::Parser::new().advance(&mut output, bytes);
    output.text
}
