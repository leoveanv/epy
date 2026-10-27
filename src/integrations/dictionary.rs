mod worker;
pub use worker::DictionaryWorker;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::{Error, Result};

pub const DICTIONARY_QUERY_BYTES_MAX: usize = 4096;

#[derive(Clone, Debug)]
pub struct ExecutableSearch {
    pub path: OsString,
    pub directory: PathBuf,
}

impl ExecutableSearch {
    pub fn capture() -> std::io::Result<Self> {
        Ok(Self {
            path: std::env::var_os("PATH").unwrap_or_default(),
            directory: std::env::current_dir()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryCommand {
    pub program: PathBuf,
    pub arguments: Vec<String>,
}

impl DictionaryCommand {
    pub fn resolve(preference: &str, search: &ExecutableSearch) -> Result<Self> {
        let words = shell_words::split(preference).map_err(|error| {
            Error::Compatibility(format!("Invalid dictionary command: {error}"))
        })?;
        if let Some(program) = words.first()
            && program != "auto"
            && let Ok(program) = which::which_in(program, Some(&search.path), &search.directory)
        {
            return Ok(Self {
                program,
                arguments: words[1..].to_vec(),
            });
        }
        for name in ["wkdict", "sdcv", "dict"] {
            if let Ok(program) = which::which_in(name, Some(&search.path), &search.directory) {
                return Ok(Self {
                    program,
                    arguments: if name == "sdcv" {
                        vec!["-n".into()]
                    } else {
                        Vec::new()
                    },
                });
            }
        }
        Err(Error::Compatibility("No dictionary client is available. Set dictionary_client or install wkdict, sdcv, or dict.".into()))
    }

    pub fn for_word(&self, word: &str) -> Self {
        let mut command = self.clone();
        command.arguments.push(word.into());
        command
    }
}

#[derive(Clone, Debug)]
pub struct DictionaryLimits {
    pub output_bytes_max: usize,
    pub timeout: Duration,
}

impl Default for DictionaryLimits {
    fn default() -> Self {
        Self {
            output_bytes_max: 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DictionaryOptions {
    pub client: String,
    pub executables: ExecutableSearch,
    pub limits: DictionaryLimits,
}

impl DictionaryOptions {
    pub fn new(client: &str, executables: ExecutableSearch) -> Self {
        Self {
            client: client.into(),
            executables,
            limits: DictionaryLimits::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DictionaryJob {
    pub request_id: u64,
    pub word: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryDefinition {
    pub word: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct DictionaryPage {
    pub title: String,
    pub lines: Vec<String>,
}

impl DictionaryPage {
    pub fn new(title: &str, text: &str, columns: usize) -> Self {
        let text = if text.trim().is_empty() {
            "No definition was returned."
        } else {
            text
        };
        Self {
            title: title.into(),
            lines: text
                .lines()
                .flat_map(|line| crate::document::text::wrap_text(line, columns.max(1)))
                .collect(),
        }
    }
}

#[derive(Debug)]
pub struct DictionaryResult {
    pub request_id: u64,
    pub result: std::result::Result<DictionaryDefinition, String>,
    pub page: DictionaryPage,
}
