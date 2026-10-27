use super::{Annotation, AnnotationBook, ExportState};

#[derive(Clone, Debug)]
pub struct ReassociationSource {
    pub book: AnnotationBook,
    pub annotation_count: usize,
}
#[derive(Clone, Debug)]
pub enum ReassociationCommand {
    Sources,
    Copy(String),
}
#[derive(Clone, Debug)]
pub struct ReassociationJob {
    pub request_id: u64,
    pub command: ReassociationCommand,
}
#[derive(Clone, Debug)]
pub struct ReassociationReport {
    pub annotations: Vec<Annotation>,
    pub copied_count: usize,
    pub skipped_count: usize,
    pub first_copied_id: Option<String>,
    pub export: ExportState,
}
#[derive(Clone, Debug)]
pub enum ReassociationReply {
    Sources(Vec<ReassociationSource>),
    Copied(ReassociationReport),
}
#[derive(Clone, Debug)]
pub struct ReassociationResult {
    pub request_id: u64,
    pub reply: std::result::Result<ReassociationReply, String>,
}
