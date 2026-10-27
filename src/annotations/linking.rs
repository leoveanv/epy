use super::{ExportLink, ExportState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub path: String,
    pub exists: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverwriteApproval {
    pub link: ExportLink,
    pub file_digest: String,
}

#[derive(Clone, Debug)]
pub enum LinkCommand {
    Status,
    Inspect(String),
    Set {
        destination: Destination,
        create: bool,
    },
    Refresh,
    PrepareOverwrite,
    Overwrite(OverwriteApproval),
    Unlink,
}

#[derive(Clone, Debug)]
pub struct LinkJob {
    pub request_id: u64,
    pub command: LinkCommand,
}

#[derive(Clone, Debug)]
pub enum LinkReply {
    Destination(Destination),
    Overwrite(OverwriteApproval),
    State(ExportState),
}

#[derive(Clone, Debug)]
pub struct LinkResult {
    pub request_id: u64,
    pub reply: std::result::Result<LinkReply, String>,
}
