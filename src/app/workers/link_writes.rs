use super::BookWorker;
use super::export::{ExportRequest, ExportWorker};
use crate::annotations::{
    Destination, ExportLink, ExportState, LinkCommand, LinkReply, OverwriteApproval,
};
use crate::export::{file_digest, validate_destination};
use crate::filesystem::sync_parent;
use crate::{Error, Result};
use std::fs::OpenOptions;

pub(super) struct LinkOutcome {
    pub reply: LinkReply,
    pub export: Option<ExportRequest>,
}

impl BookWorker {
    pub(super) fn apply_link(
        &mut self,
        command: LinkCommand,
        exports: &ExportWorker,
    ) -> Result<LinkOutcome> {
        let book_id = self.annotation_book()?.id.clone();
        match command {
            LinkCommand::Status => Ok(LinkOutcome {
                reply: LinkReply::State(ExportState {
                    link: self.store.export_link(&book_id)?,
                    ..ExportState::default()
                }),
                export: None,
            }),
            LinkCommand::Inspect(value) => {
                let path = validate_destination(&value)?;
                let path_text = path
                    .to_str()
                    .ok_or_else(|| {
                        Error::Compatibility(
                            "The resolved Markdown path must be valid Unicode.".into(),
                        )
                    })?
                    .to_owned();
                Ok(LinkOutcome {
                    reply: LinkReply::Destination(Destination {
                        exists: path.exists(),
                        path: path_text,
                    }),
                    export: None,
                })
            }
            LinkCommand::PrepareOverwrite => {
                exports.cancel_and_wait()?;
                let link = self.required_link()?;
                let digest = file_digest(&std::fs::read(validate_destination(&link.path)?)?);
                Ok(LinkOutcome {
                    reply: LinkReply::Overwrite(OverwriteApproval {
                        link,
                        file_digest: digest,
                    }),
                    export: None,
                })
            }
            LinkCommand::Unlink => {
                exports.cancel_and_wait()?;
                self.store.unlink(&book_id)?;
                Ok(LinkOutcome {
                    reply: LinkReply::State(ExportState::default()),
                    export: None,
                })
            }
            LinkCommand::Refresh => {
                exports.cancel_and_wait()?;
                let link = self.required_link()?;
                Ok(queued_export(ExportRequest::checked(link)))
            }
            LinkCommand::Overwrite(approval) => {
                exports.cancel_and_wait()?;
                if self.required_link()? != approval.link {
                    return Err(Error::Compatibility(
                        "The Markdown link or annotations changed. Review and confirm again."
                            .into(),
                    ));
                }
                Ok(queued_export(ExportRequest {
                    link: approval.link.clone(),
                    overwrite: Some(approval),
                }))
            }
            LinkCommand::Set {
                destination,
                create,
            } => {
                exports.cancel_and_wait()?;
                let path = validate_destination(&destination.path)?;
                if path.to_str() != Some(&destination.path) || path.exists() != destination.exists {
                    return Err(Error::Compatibility(
                        "The resolved destination changed. Review it again.".into(),
                    ));
                }
                if !destination.exists {
                    if !create {
                        return Err(Error::Compatibility(
                            "Creating the missing Markdown file requires confirmation.".into(),
                        ));
                    }
                    let file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)?;
                    file.sync_all()?;
                    sync_parent(&path)?;
                }
                let previous = self.store.export_link(&book_id)?;
                let link = match previous {
                    Some(link) if link.path == destination.path && destination.exists => link,
                    previous => ExportLink {
                        book_id: book_id.clone(),
                        path: destination.path,
                        digest: None,
                        revision: previous.map_or(0, |link| link.revision),
                        exported_revision: -1,
                    },
                };
                self.store.set_link(&link)?;
                Ok(queued_export(ExportRequest::checked(link)))
            }
        }
    }

    fn required_link(&self) -> Result<ExportLink> {
        self.store
            .export_link(&self.annotation_book()?.id)?
            .ok_or_else(|| {
                Error::Compatibility(
                    "This book has no Markdown link. Open O and choose a destination.".into(),
                )
            })
    }
}

fn queued_export(request: ExportRequest) -> LinkOutcome {
    LinkOutcome {
        reply: LinkReply::State(ExportState {
            running: true,
            link: Some(request.link.clone()),
            error: None,
        }),
        export: Some(request),
    }
}
