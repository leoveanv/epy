mod process;

use super::{
    DICTIONARY_QUERY_BYTES_MAX, DictionaryCommand, DictionaryDefinition, DictionaryJob,
    DictionaryOptions, DictionaryPage, DictionaryResult,
};
use crate::{Error, Result};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

#[derive(Default)]
struct WorkerState {
    generation: u64,
    result: Option<DictionaryResult>,
}

impl WorkerState {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.result = None;
        self.generation
    }
}

pub struct DictionaryWorker {
    options: DictionaryOptions,
    runtime: Option<Runtime>,
    active: Mutex<Option<JoinHandle<()>>>,
    state: Arc<Mutex<WorkerState>>,
}

impl DictionaryWorker {
    pub fn start(options: DictionaryOptions) -> Result<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .enable_all()
            .build()?;
        Ok(Self {
            options,
            runtime: Some(runtime),
            active: Mutex::new(None),
            state: Arc::new(Mutex::new(WorkerState::default())),
        })
    }

    pub fn request(&self, job: DictionaryJob, columns: usize) {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(task) = active.take() {
            task.abort();
        }
        let generation = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .advance();
        let state = self.state.clone();
        let options = self.options.clone();
        *active = Some(
            self.runtime
                .as_ref()
                .expect("The dictionary runtime is owned until Drop")
                .spawn(async move {
                    let request_id = job.request_id;
                    let word = job.word.clone();
                    let result = lookup(options, job)
                        .await
                        .map_err(|error| error.to_string());
                    let page = match &result {
                        Ok(definition) => DictionaryPage::new(
                            &format!("Definition: {}", word.to_uppercase()),
                            &definition.text,
                            columns,
                        ),
                        Err(message) => DictionaryPage::new("Error: Dictionary", message, columns),
                    };
                    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                    if generation == state.generation {
                        state.result = Some(DictionaryResult {
                            request_id,
                            result,
                            page,
                        });
                    }
                }),
        );
    }

    pub fn cancel(&self) {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(task) = active.take() {
            task.abort();
        }
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .advance();
    }

    pub fn take_result(&self) -> Option<DictionaryResult> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .result
            .take()
    }
}

// #NOTE: Abort drops the owned process-group guard; bounded runtime shutdown lets that cleanup run before the reader exits.
impl Drop for DictionaryWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_millis(50));
        }
    }
}

async fn lookup(options: DictionaryOptions, job: DictionaryJob) -> Result<DictionaryDefinition> {
    if job.word.trim().is_empty()
        || job.word.len() > DICTIONARY_QUERY_BYTES_MAX
        || job.word.chars().any(char::is_control)
    {
        return Err(Error::Compatibility(
            "Enter a dictionary query of at most 4096 bytes without control characters.".into(),
        ));
    }
    let command =
        DictionaryCommand::resolve(&options.client, &options.executables)?.for_word(&job.word);
    let text = process::definition(&command, &options).await?;
    Ok(DictionaryDefinition {
        word: job.word,
        text,
    })
}
