use super::{Annotation, ExportLink, TextAnchor};

#[derive(Clone, Debug)]
pub struct AnnotationDraft {
    pub anchor: TextAnchor,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct NoteUpdate {
    pub annotation_id: String,
    pub note: String,
}

#[derive(Clone, Debug)]
pub enum AnnotationCommand {
    Create(AnnotationDraft),
    EditNote(NoteUpdate),
    Delete(String),
}

#[derive(Clone, Debug)]
pub struct AnnotationJob {
    pub request_id: u64,
    pub command: AnnotationCommand,
}

#[derive(Clone, Debug, Default)]
pub struct ExportState {
    pub running: bool,
    pub link: Option<ExportLink>,
    pub error: Option<String>,
}

impl ExportState {
    pub fn pending(&self) -> bool {
        self.running
            || self.error.is_some()
            || self
                .link
                .as_ref()
                .is_some_and(|link| link.revision != link.exported_revision)
    }
}

#[derive(Clone, Debug)]
pub struct AnnotationResult {
    pub request_id: u64,
    pub annotations: Vec<Annotation>,
    pub saved_id: Option<String>,
    pub error: Option<String>,
    pub export: ExportState,
}
