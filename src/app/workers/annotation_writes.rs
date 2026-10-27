use std::sync::Arc;

use crate::annotations::references::{ReferenceRequest, resolve_reference};
use crate::annotations::{
    Annotation, AnnotationCommand, AnnotationJob, AnnotationResult, ExportState,
};
use crate::annotations::{AnnotationReference, TextAnchor};
use crate::document::text::{ParsedDocument, parse_document};
use crate::{Error, Result};

use super::BookWorker;

impl BookWorker {
    pub(super) fn apply_annotation(&mut self, job: AnnotationJob) -> AnnotationResult {
        let result = self.commit_annotation(&job.command);
        let (saved_id, error) = match result {
            Ok(saved) => {
                let records = &mut Arc::make_mut(&mut self.info).annotations;
                match saved {
                    Some(annotation) => {
                        let id = annotation.id.clone();
                        records.retain(|record| record.id != id);
                        records.push(annotation);
                        (Some(id), None)
                    }
                    None => {
                        if let AnnotationCommand::Delete(id) = &job.command {
                            records.retain(|record| &record.id != id);
                        }
                        (None, None)
                    }
                }
            }
            Err(error) => (None, Some(error.to_string())),
        };
        Arc::make_mut(&mut self.info)
            .annotations
            .sort_by(|left, right| {
                (left.content_order, left.anchor.start(), &left.id).cmp(&(
                    right.content_order,
                    right.anchor.start(),
                    &right.id,
                ))
            });
        let export = self.export_state();
        AnnotationResult {
            request_id: job.request_id,
            annotations: self.info.annotations.clone(),
            saved_id,
            error,
            export,
        }
    }

    pub(super) fn export_state(&self) -> ExportState {
        let Some(book) = &self.annotation_book else {
            return ExportState::default();
        };
        match self.store.export_link(&book.id) {
            Ok(link) => ExportState {
                link,
                ..ExportState::default()
            },
            Err(error) => ExportState {
                link: None,
                error: Some(error.to_string()),
                ..ExportState::default()
            },
        }
    }

    pub(super) fn reference_for(
        &self,
        anchor: &TextAnchor,
        document: &ParsedDocument,
        content_order: usize,
    ) -> Result<AnnotationReference> {
        let navigation = self.book.navigation();
        let fallback = navigation
            .sections
            .iter()
            .rfind(|target| {
                self.info
                    .contents
                    .iter()
                    .position(|content| *content == target.content_id)
                    .is_some_and(|order| order < content_order)
            })
            .map_or(&[][..], |target| target.headings.as_slice());
        resolve_reference(ReferenceRequest {
            anchor,
            document,
            content_order,
            navigation,
            fallback_headings: fallback,
        })
    }

    fn commit_annotation(&mut self, command: &AnnotationCommand) -> Result<Option<Annotation>> {
        let book_id = self.annotation_book()?.id.clone();
        match command {
            AnnotationCommand::Create(draft) => {
                let content_order = self
                    .info
                    .contents
                    .iter()
                    .position(|content| *content == draft.anchor.content_id())
                    .ok_or_else(|| {
                        Error::Compatibility("The annotation document is not in this ebook.".into())
                    })?;
                let parsed = parse_document(
                    draft.anchor.content_id(),
                    &self.book.raw_text(draft.anchor.content_id())?,
                )?;
                if draft.anchor.resolve(&parsed.source).is_none() {
                    return Err(Error::Compatibility(
                        "The selected quotation no longer matches this document.".into(),
                    ));
                }
                let reference = self.reference_for(&draft.anchor, &parsed, content_order)?;
                let chapter = reference.headings.join(" / ");
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
                let annotation = Annotation {
                    id: uuid::Uuid::new_v4().to_string(),
                    book_id: book_id.clone(),
                    anchor: draft.anchor.clone(),
                    note: draft.note.clone(),
                    chapter,
                    reference: Some(reference),
                    content_order,
                    created_at: now.clone(),
                    updated_at: now,
                };
                self.store.save_annotation(&annotation)?;
                Ok(Some(annotation))
            }
            AnnotationCommand::EditNote(update) => {
                let mut annotation = self
                    .store
                    .annotations(&book_id)?
                    .into_iter()
                    .find(|annotation| annotation.id == update.annotation_id)
                    .ok_or_else(|| {
                        Error::Compatibility("This annotation no longer exists.".into())
                    })?;
                annotation.note = update.note.clone();
                annotation.updated_at =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
                self.store.save_annotation(&annotation)?;
                Ok(Some(annotation))
            }
            AnnotationCommand::Delete(id) => {
                let annotation = self
                    .store
                    .annotations(&book_id)?
                    .into_iter()
                    .find(|annotation| &annotation.id == id)
                    .ok_or_else(|| {
                        Error::Compatibility("This annotation no longer exists.".into())
                    })?;
                self.store.delete_annotation(&annotation)?;
                Ok(None)
            }
        }
    }
}
