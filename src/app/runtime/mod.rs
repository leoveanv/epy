mod effects;
mod events;
mod retirement;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use crate::Result;
use crate::app::workers::ReaderJobs;
use crate::app::{BookSessionMemory, ReaderModel, Viewport};
use crate::book::BookLocation;
use crate::configuration::{Configuration, InputKey};
use crate::reading::ReadingState;
use retirement::RetiringJobs;

pub struct ReaderLaunch {
    pub image_decoder: PathBuf,
    pub executables: crate::integrations::ExecutableSearch,
    pub location: BookLocation,
    pub database: PathBuf,
    pub configuration: Configuration,
    pub viewport: Viewport,
}

pub struct ReaderRuntime {
    pub model: Option<ReaderModel>,
    jobs: ReaderJobs,
    opening: Option<ReaderJobs>,
    memories: BTreeMap<String, BookSessionMemory>,
    retiring: RetiringJobs,
    database: PathBuf,
    configuration: Configuration,
    viewport: Viewport,
    submitted: Option<SubmittedPosition>,
    finished: bool,
    browser_requests: VecDeque<url::Url>,
    viewer_requests: VecDeque<std::sync::Arc<crate::images::ImagePayload>>,
    images: Option<crate::images::ImageWorker>,
    image_decoder: PathBuf,
    executables: crate::integrations::ExecutableSearch,
    dictionary: Option<crate::integrations::DictionaryWorker>,
    pending_notice: Option<IntegrationNotice>,
}

struct IntegrationNotice {
    path: String,
    message: String,
}

struct SubmittedPosition {
    path: String,
    position: ReadingState,
}

impl ReaderRuntime {
    pub fn start(launch: ReaderLaunch) -> Self {
        let spread = if launch.configuration.settings.start_with_double_spread {
            2
        } else {
            1
        };
        let width = launch.viewport.available_width(spread);
        Self {
            jobs: ReaderJobs::start_with_options(
                launch.location,
                launch.database.clone(),
                crate::app::workers::ReaderJobOptions {
                    width,
                    inline_images: launch.configuration.settings.inline_images,
                },
            ),
            model: None,
            opening: None,
            memories: BTreeMap::new(),
            retiring: RetiringJobs::default(),
            database: launch.database,
            configuration: launch.configuration,
            viewport: launch.viewport,
            submitted: None,
            finished: false,
            browser_requests: VecDeque::new(),
            viewer_requests: VecDeque::new(),
            images: None,
            image_decoder: launch.image_decoder,
            executables: launch.executables,
            dictionary: None,
            pending_notice: None,
        }
    }

    pub(crate) fn book_location(&self) -> &BookLocation {
        self.jobs.location()
    }

    pub fn take_browser_request(&mut self) -> Option<url::Url> {
        self.browser_requests.pop_front()
    }

    pub fn take_viewer_request(&mut self) -> Option<std::sync::Arc<crate::images::ImagePayload>> {
        self.viewer_requests.pop_front()
    }

    pub fn integration_completed(&mut self, result: std::result::Result<(), String>) {
        if let Err(message) = result
            && let Some(model) = &self.model
        {
            self.pending_notice = Some(IntegrationNotice {
                path: model.info.path.clone(),
                message,
            });
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub(crate) fn shutdown(&mut self) {
        self.cancel_open();
        self.jobs.cancel_search();
        let Some(model) = &mut self.model else {
            self.finished = true;
            return;
        };
        if self.finished || model.quitting {
            return;
        }
        let request = model.save_request(true);
        model.quitting = true;
        self.apply_effects(vec![crate::app::ReaderEffect::Quit(request)]);
    }

    pub fn poll(&mut self) -> Result<bool> {
        let mut changed = false;
        for event in self.jobs.events.try_iter().collect::<Vec<_>>() {
            changed = true;
            self.active_event(event)?;
        }
        if let Some(result) = self
            .images
            .as_ref()
            .and_then(crate::images::ImageWorker::take_result)
            && let Some(model) = &mut self.model
        {
            model.image_completed(result);
            changed = true;
        }
        if let Some(result) = self
            .dictionary
            .as_ref()
            .and_then(crate::integrations::DictionaryWorker::take_result)
            && let Some(model) = &mut self.model
        {
            model.dictionary_completed(result);
            changed = true;
        }
        while let Some(opening) = &self.opening {
            let Ok(event) = opening.events.try_recv() else {
                break;
            };
            changed = true;
            self.opening_event(event);
        }
        if let Some(model) = &mut self.model
            && !model.loading
            && let Some(notice) = self.pending_notice.take()
            && notice.path == model.info.path
        {
            model.status = notice.message;
            changed = true;
        }
        Ok(changed)
    }

    pub fn handle_key(&mut self, key: InputKey) {
        if let Some(model) = &mut self.model {
            let effects = model.handle_key(key);
            self.apply_effects(effects);
        } else if matches!(key, InputKey::Escape | InputKey::Character('q' | '\u{03}')) {
            self.finished = true;
        }
    }

    pub(crate) fn handle_mouse(&mut self, action: crate::app::mouse::MouseAction) {
        if let Some(model) = &mut self.model {
            let effects = model.handle_mouse(action);
            self.apply_effects(effects);
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if let Some(model) = &mut self.model {
            model.handle_paste(text);
        }
    }

    pub fn resize(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        if let Some(model) = &mut self.model {
            let effects = model.resize(viewport);
            self.apply_effects(effects);
        }
    }

    pub fn autosave(&mut self) {
        let Some(model) = &self.model else {
            return;
        };
        if !model.can_autosave() {
            return;
        }
        let position = model.position();
        if self.submitted.as_ref().is_some_and(|submitted| {
            submitted.path == model.info.path && submitted.position == position
        }) {
            return;
        }
        self.submitted = Some(SubmittedPosition {
            path: model.info.path.clone(),
            position,
        });
        self.jobs.save(model.save_request(false));
    }

    fn begin_open(&mut self, path: String) {
        if !self.retiring.available() {
            self.open_failed("Previous books are still closing. Wait briefly and retry.".into());
            return;
        }
        let location = match BookLocation::parse(&path) {
            Ok(location) => location,
            Err(error) => {
                self.open_failed(error.to_string());
                return;
            }
        };
        let spread = self.memories.get(&path).map_or_else(
            || {
                if self.configuration.settings.start_with_double_spread {
                    2
                } else {
                    1
                }
            },
            BookSessionMemory::spread,
        );
        let width = self.viewport.available_width(spread);
        self.opening = Some(ReaderJobs::start_with_options(
            location,
            self.database.clone(),
            crate::app::workers::ReaderJobOptions {
                width,
                inline_images: self.configuration.settings.inline_images,
            },
        ));
    }

    fn cancel_open(&mut self) {
        if let Some(opening) = self.opening.take() {
            self.retiring.retire(opening);
        }
    }

    fn open_failed(&mut self, message: String) {
        self.cancel_open();
        if let Some(model) = &mut self.model {
            model.library_open_failed(message);
            let effects = model.resize(self.viewport);
            self.apply_effects(effects);
        }
    }
}

impl Drop for ReaderRuntime {
    fn drop(&mut self) {
        self.jobs.stop();
        if let Some(opening) = &self.opening {
            opening.stop();
        }
    }
}
