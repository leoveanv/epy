use super::ReaderRuntime;
use crate::app::workers::WorkerEvent;
use crate::app::{ReaderModel, ReaderSession};
use crate::{Error, Result};

impl ReaderRuntime {
    pub(super) fn active_event(&mut self, event: WorkerEvent) -> Result<()> {
        if let WorkerEvent::Opened {
            info,
            rendered,
            position,
        } = event
        {
            let initial_position = position.clone().into();
            let mut model = ReaderModel::new(
                ReaderSession {
                    info,
                    rendered,
                    position,
                    configuration: self.configuration.clone(),
                },
                self.viewport,
            );
            let effects = model.initial_reading(initial_position);
            self.model = Some(model);
            self.apply_effects(effects);
            return Ok(());
        }
        let Some(model) = &mut self.model else {
            return match event {
                WorkerEvent::Failed(message) => Err(Error::Compatibility(message)),
                _ => Ok(()),
            };
        };
        let effects = match event {
            WorkerEvent::Loaded {
                generation,
                rendered,
                request,
            } if self.jobs.is_current(generation) => model.accept_loaded(rendered, request),
            WorkerEvent::WindowLoaded { generation, view } if self.jobs.is_current(generation) => {
                model.accept_window(view)
            }
            WorkerEvent::LoadFailed {
                generation,
                message,
            } if self.jobs.is_current(generation) => model.load_failed(message),
            WorkerEvent::ExternalLink { generation, url } if self.jobs.is_current(generation) => {
                model.external_link_ready(url)
            }
            WorkerEvent::Search(result) => model.search_completed(result),
            WorkerEvent::Library(result) => model.library_completed(result),
            WorkerEvent::Annotation(result) => model.annotation_completed(result),
            WorkerEvent::Link(result) => model.link_completed(result),
            WorkerEvent::Reassociation(result) => model.reassociation_completed(result),
            WorkerEvent::Totals(totals) => {
                model.set_totals(totals);
                Vec::new()
            }
            WorkerEvent::Export(state) => {
                model.export_updated(state);
                Vec::new()
            }
            WorkerEvent::Bookmarks(bookmarks) => {
                model.bookmarks_saved(bookmarks);
                Vec::new()
            }
            WorkerEvent::Saved { quitting, error } => {
                if let Some(error) = error {
                    self.submitted = None;
                    model.quitting = false;
                    model.status = format!("Save failed: {error}");
                } else if quitting {
                    self.finished = true;
                }
                Vec::new()
            }
            WorkerEvent::Failed(message) => {
                model.status = message;
                Vec::new()
            }
            _ => Vec::new(),
        };
        self.apply_effects(effects);
        Ok(())
    }

    pub(super) fn opening_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Opened {
                info,
                rendered,
                position,
            } => {
                let Some(opening) = self.opening.take() else {
                    return;
                };
                let mut initial_position = position.clone().into();
                let mut model = ReaderModel::new(
                    ReaderSession {
                        info,
                        rendered,
                        position,
                        configuration: self.configuration.clone(),
                    },
                    self.viewport,
                );
                if let Some(images) = &self.images {
                    images.cancel();
                }
                if let Some(dictionary) = &self.dictionary {
                    dictionary.cancel();
                }
                if let Some(previous) = self.model.take() {
                    self.memories
                        .insert(previous.info.path.clone(), previous.remember_session());
                }
                if let Some(memory) = self.memories.remove(&model.info.path)
                    && let Some(position) = model.restore_session(memory)
                {
                    initial_position = position;
                }
                self.retiring
                    .retire(std::mem::replace(&mut self.jobs, opening));
                self.submitted = None;
                let effects = model.initial_reading(initial_position);
                self.model = Some(model);
                self.apply_effects(effects);
            }
            WorkerEvent::Failed(message) => self.open_failed(message),
            _ => {}
        }
    }
}
