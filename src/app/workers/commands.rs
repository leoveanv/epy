use super::export::{ExportRequest, ExportWorker};
use super::{BookWorker, Mailbox, WorkCommand, WorkerEvent, send_event};
use crate::annotations::{LinkResult, ReassociationReply, ReassociationResult};
use crossbeam_channel::Sender;

pub(super) struct WorkerServices<'a> {
    pub sender: &'a Sender<WorkerEvent>,
    pub mailbox: &'a Mailbox,
    pub exports: Option<&'a ExportWorker>,
}

pub(super) fn run_command(
    worker: &mut BookWorker,
    command: WorkCommand,
    services: WorkerServices<'_>,
) {
    match command {
        WorkCommand::Annotation(job) => {
            let result = worker.apply_annotation(job);
            let request = result
                .export
                .link
                .clone()
                .filter(|_| result.error.is_none() && result.export.pending());
            send_event(
                services.sender,
                services.mailbox,
                WorkerEvent::Annotation(result),
            );
            if let Some(link) = request
                && let Some(exports) = services.exports
            {
                exports.request(ExportRequest::checked(link));
            }
        }
        WorkCommand::Link(job) => {
            let outcome = services
                .exports
                .ok_or_else(|| {
                    crate::Error::Compatibility(
                        "Annotations are available only for local EPUB books.".into(),
                    )
                })
                .and_then(|exports| worker.apply_link(job.command, exports));
            let (reply, export) = match outcome {
                Ok(outcome) => (Ok(outcome.reply), outcome.export),
                Err(error) => (Err(error.to_string()), None),
            };
            send_event(
                services.sender,
                services.mailbox,
                WorkerEvent::Link(LinkResult {
                    request_id: job.request_id,
                    reply,
                }),
            );
            if let Some(request) = export
                && let Some(exports) = services.exports
            {
                exports.request(request);
            }
        }
        WorkCommand::Reassociation(job) => {
            let reply = worker
                .reassociate(job.command)
                .map_err(|error| error.to_string());
            let export = match &reply {
                Ok(ReassociationReply::Copied(report))
                    if report.copied_count > 0 && report.export.pending() =>
                {
                    report.export.link.clone()
                }
                _ => None,
            };
            send_event(
                services.sender,
                services.mailbox,
                WorkerEvent::Reassociation(ReassociationResult {
                    request_id: job.request_id,
                    reply,
                }),
            );
            if let Some(link) = export
                && let Some(exports) = services.exports
            {
                exports.request(ExportRequest::checked(link));
            }
        }
        WorkCommand::Library(job) => {
            let reply = worker
                .library(job.command)
                .map_err(|error| error.to_string());
            send_event(
                services.sender,
                services.mailbox,
                WorkerEvent::Library(crate::app::requests::LibraryResult {
                    request_id: job.request_id,
                    reply,
                }),
            );
        }
        WorkCommand::Bookmark(bookmark) => {
            let result = worker
                .store
                .insert_bookmark(&worker.info.path, &bookmark.name, &bookmark.position)
                .and_then(|()| worker.store.bookmarks(&worker.info.path));
            let event = match result {
                Ok(bookmarks) => WorkerEvent::Bookmarks(bookmarks),
                Err(error) => WorkerEvent::Failed(error.to_string()),
            };
            send_event(services.sender, services.mailbox, event);
        }
    }
}
