use super::BookWorker;
use crate::annotations::{
    Annotation, ReassociationCommand, ReassociationReply, ReassociationReport,
};
use crate::document::text::parse_document;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::sync::Arc;

impl BookWorker {
    pub(super) fn reassociate(
        &mut self,
        command: ReassociationCommand,
    ) -> Result<ReassociationReply> {
        let book_id = self.annotation_book()?.id.clone();
        match command {
            ReassociationCommand::Sources => Ok(ReassociationReply::Sources(
                self.store.reassociation_sources(&book_id)?,
            )),
            ReassociationCommand::Copy(source_id) => self
                .copy_annotations(&source_id)
                .map(ReassociationReply::Copied),
        }
    }

    fn copy_annotations(&mut self, source_id: &str) -> Result<ReassociationReport> {
        let book_id = self.annotation_book()?.id.clone();
        if source_id == book_id
            || !self
                .store
                .reassociation_sources(&book_id)?
                .iter()
                .any(|source| source.book.id == source_id)
        {
            return Err(Error::Compatibility(
                "Choose a different annotated book identity before copying.".into(),
            ));
        }
        let mut groups = BTreeMap::<String, Vec<Annotation>>::new();
        for annotation in self.store.annotations(source_id)? {
            groups
                .entry(annotation.anchor.content_id().into())
                .or_default()
                .push(annotation);
        }
        let mut copies = Vec::new();
        let mut skipped_count = 0;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
        for (content_id, annotations) in groups {
            let Some(content_order) = self
                .info
                .contents
                .iter()
                .position(|content| *content == content_id)
            else {
                skipped_count += annotations.len();
                continue;
            };
            let document = parse_document(&content_id, &self.book.raw_text(&content_id)?)?;
            for annotation in annotations {
                if annotation.anchor.resolve(&document.source).is_none() {
                    skipped_count += 1;
                    continue;
                }
                let reference = self.reference_for(&annotation.anchor, &document, content_order)?;
                copies.push(Annotation {
                    id: uuid::Uuid::new_v4().to_string(),
                    book_id: book_id.clone(),
                    anchor: annotation.anchor,
                    note: annotation.note,
                    chapter: reference.headings.join(" / "),
                    reference: Some(reference),
                    content_order,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                });
            }
        }
        self.store.insert_annotations(&copies)?;
        let copied_count = copies.len();
        let first_copied_id = copies.first().map(|annotation| annotation.id.clone());
        let records = &mut Arc::make_mut(&mut self.info).annotations;
        records.extend(copies);
        records.sort_by(|left, right| {
            (left.content_order, left.anchor.start(), &left.id).cmp(&(
                right.content_order,
                right.anchor.start(),
                &right.id,
            ))
        });
        Ok(ReassociationReport {
            annotations: records.clone(),
            copied_count,
            skipped_count,
            first_copied_id,
            export: self.export_state(),
        })
    }
}
