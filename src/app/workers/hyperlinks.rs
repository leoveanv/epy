use super::book::BookWorker;
use crate::book::{BookKind, ResourceTarget};
use crate::document::RenderedDocument;
use crate::reading::LoadRequest;
use crate::reading::ReadingState;
use crate::reading::{HyperlinkRequest, external_url};
use crate::{Error, Result};

pub(super) enum HyperlinkDestination {
    Internal(LoadRequest),
    External(url::Url),
}

impl BookWorker {
    pub(super) fn resolve_hyperlink(
        &mut self,
        request: &HyperlinkRequest,
        active: &dyn Fn() -> bool,
    ) -> Result<HyperlinkDestination> {
        if let Some(url) = external_url(&request.link.content_id, &request.link.href)? {
            return Ok(HyperlinkDestination::External(url));
        }
        let target = self
            .book
            .resolve_resource(&request.link.content_id, &request.link.href)?;
        let position = self.hyperlink_position(&target, request.width, active)?;
        let index = self
            .info
            .contents
            .iter()
            .position(|id| *id == position.content_id)
            .ok_or_else(|| {
                Error::Compatibility("The linked document is not in this ebook.".into())
            })?;
        Ok(HyperlinkDestination::Internal(LoadRequest {
            index,
            width: request.width,
            position: Some(position.into()),
            at_end: false,
            fragment: None,
        }))
    }

    fn hyperlink_position(
        &mut self,
        target: &ResourceTarget,
        width: usize,
        active: &dyn Fn() -> bool,
    ) -> Result<ReadingState> {
        if let Some(fragment) = &target.fragment {
            let candidates = if self.book.kind() == BookKind::FictionBook {
                0..self.info.contents.len()
            } else {
                let index = self
                    .info
                    .contents
                    .iter()
                    .position(|id| *id == target.content_id)
                    .ok_or_else(|| {
                        Error::Compatibility("The linked document is not in this ebook.".into())
                    })?;
                index..index + 1
            };
            let mut destination = None;
            for index in candidates {
                if !active() {
                    return Err(Error::Canceled);
                }
                let document = self.load(&LoadRequest {
                    index,
                    width,
                    position: None,
                    fragment: None,
                    at_end: false,
                })?;
                if let Some(position) = anchor_position(&document, fragment)?
                    && destination.replace(position).is_some()
                {
                    return Err(fragment_error(fragment));
                }
            }
            destination.ok_or_else(|| fragment_error(fragment))
        } else {
            Ok(ReadingState {
                content_id: target.content_id.clone(),
                textwidth: width,
                ..ReadingState::default()
            })
        }
    }
}

fn anchor_position(document: &RenderedDocument, fragment: &str) -> Result<Option<ReadingState>> {
    let mut anchors = document
        .parsed
        .semantics
        .anchors
        .iter()
        .filter(|anchor| anchor.fragment == fragment);
    let Some(anchor) = anchors.next() else {
        return Ok(None);
    };
    if anchors.next().is_some() {
        return Err(fragment_error(fragment));
    }
    Ok(Some(ReadingState {
        content_id: document.parsed.source.content_id.clone(),
        textwidth: document.width,
        offset: anchor.offset,
        screen_row: 0,
    }))
}

fn fragment_error(fragment: &str) -> Error {
    Error::Compatibility(format!(
        "The link destination is missing or ambiguous: {fragment}"
    ))
}
