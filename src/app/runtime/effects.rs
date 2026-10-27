use super::ReaderRuntime;
use crate::annotations::{AnnotationResult, LinkResult, ReassociationResult};
use crate::app::ReaderEffect;
use crate::app::requests::LibraryResult;
use std::collections::VecDeque;

impl ReaderRuntime {
    pub(super) fn apply_effects(&mut self, effects: Vec<ReaderEffect>) {
        let mut pending = VecDeque::from(effects);
        while let Some(effect) = pending.pop_front() {
            let more = match effect {
                ReaderEffect::Load(request) => {
                    self.jobs.load(request);
                    Vec::new()
                }
                ReaderEffect::Window(request) => {
                    self.jobs.prepare_window(request);
                    Vec::new()
                }
                ReaderEffect::Hyperlink(request) => {
                    self.jobs.follow_hyperlink(request);
                    Vec::new()
                }
                ReaderEffect::Browser(url) => {
                    self.browser_requests.push_back(url);
                    Vec::new()
                }
                ReaderEffect::Image(job) => {
                    self.images
                        .get_or_insert_with(|| {
                            crate::images::ImageWorker::start(
                                crate::images::ImageWorkerOptions::new(self.image_decoder.clone()),
                            )
                        })
                        .request(self.jobs.location().clone(), job);
                    Vec::new()
                }
                ReaderEffect::CancelImage => {
                    self.viewer_requests.clear();
                    if let Some(images) = &self.images {
                        images.cancel();
                    }
                    Vec::new()
                }
                ReaderEffect::Viewer(payload) => {
                    self.viewer_requests.push_back(payload);
                    Vec::new()
                }
                ReaderEffect::Dictionary(job) => {
                    self.request_dictionary(job);
                    Vec::new()
                }
                ReaderEffect::CancelDictionary => {
                    if let Some(dictionary) = &self.dictionary {
                        dictionary.cancel();
                    }
                    Vec::new()
                }
                ReaderEffect::Search(job) => {
                    self.jobs.search(job);
                    Vec::new()
                }
                ReaderEffect::CancelSearch => {
                    self.jobs.cancel_search();
                    Vec::new()
                }
                ReaderEffect::Quit(request) => {
                    if let Some(dictionary) = &self.dictionary {
                        dictionary.cancel();
                    }
                    if let Some(images) = &self.images {
                        images.cancel();
                    }
                    self.jobs.save(request);
                    Vec::new()
                }
                ReaderEffect::OpenBook(path) => {
                    self.begin_open(path);
                    Vec::new()
                }
                ReaderEffect::CancelBookOpen => {
                    self.cancel_open();
                    Vec::new()
                }
                ReaderEffect::ForgetBookSession(path) => {
                    self.memories.remove(&path);
                    Vec::new()
                }
                other => self.write_effect(other),
            };
            pending.extend(more);
        }
    }

    fn request_dictionary(&mut self, job: crate::integrations::DictionaryJob) {
        use crate::integrations::{
            DictionaryOptions, DictionaryPage, DictionaryResult, DictionaryWorker,
        };
        let columns = self.viewport.columns.saturating_sub(6).max(1);
        if self.dictionary.is_none() {
            let options = DictionaryOptions::new(
                &self.configuration.settings.dictionary_client,
                self.executables.clone(),
            );
            match DictionaryWorker::start(options) {
                Ok(worker) => self.dictionary = Some(worker),
                Err(error) => {
                    if let Some(model) = &mut self.model {
                        let message = error.to_string();
                        model.dictionary_completed(DictionaryResult {
                            request_id: job.request_id,
                            page: DictionaryPage::new("Error: Dictionary", &message, columns),
                            result: Err(message),
                        });
                    }
                    return;
                }
            }
        }
        if let Some(dictionary) = &self.dictionary {
            dictionary.request(job, columns);
        }
    }

    fn write_effect(&mut self, effect: ReaderEffect) -> Vec<ReaderEffect> {
        let Some(model) = &mut self.model else {
            return Vec::new();
        };
        match effect {
            ReaderEffect::Library(job) => {
                let request_id = job.request_id;
                if let Err(error) = self.jobs.library(job) {
                    return model.library_completed(LibraryResult {
                        request_id,
                        reply: Err(error.to_string()),
                    });
                }
            }
            ReaderEffect::Annotation(job) => {
                let request_id = job.request_id;
                if let Err(error) = self.jobs.annotate(job) {
                    return model.annotation_completed(AnnotationResult {
                        request_id,
                        annotations: model.info.annotations.clone(),
                        saved_id: None,
                        error: Some(error.to_string()),
                        export: model.export_state().clone(),
                    });
                }
            }
            ReaderEffect::Link(job) => {
                let request_id = job.request_id;
                if let Err(error) = self.jobs.link(job) {
                    return model.link_completed(LinkResult {
                        request_id,
                        reply: Err(error.to_string()),
                    });
                }
            }
            ReaderEffect::Reassociation(job) => {
                let request_id = job.request_id;
                if let Err(error) = self.jobs.reassociate(job) {
                    return model.reassociation_completed(ReassociationResult {
                        request_id,
                        reply: Err(error.to_string()),
                    });
                }
            }
            ReaderEffect::Bookmark(request) => {
                if let Err(error) = self.jobs.add_bookmark(request) {
                    model.status = error.to_string();
                }
            }
            _ => unreachable!("non-write effects are handled by the runtime dispatcher"),
        }
        Vec::new()
    }
}
