use super::{ModeContext, ModeInput, ReaderEffect, ReaderMode};
use crate::configuration::{Command, InputKey};
use crate::images::{ImageJob, ImagePayload, ImageResult, ImageTarget};
use std::sync::Arc;

#[derive(Default)]
pub(in crate::app) struct ImageState {
    requests: super::RequestSequence,
}

impl ImageState {
    pub(in crate::app) fn begin(
        context: ModeContext<'_>,
        targets: Vec<ImageTarget>,
    ) -> Vec<ReaderEffect> {
        if targets.is_empty() {
            *context.status = "No visible images.".into();
            return Vec::new();
        }
        *context.mode = ReaderMode::Images(Box::new(ImageMode::Selecting(ImageSelection {
            targets,
            selected: 0,
        })));
        Vec::new()
    }

    pub(in crate::app) fn key(
        &mut self,
        context: ModeContext<'_>,
        input: ModeInput,
    ) -> Vec<ReaderEffect> {
        let ReaderMode::Images(mode) = std::mem::replace(context.mode, ReaderMode::Reading) else {
            return Vec::new();
        };
        if matches!(input.key, InputKey::Escape | InputKey::Character('\u{03}'))
            || matches!(input.command, Some(Command::Quit | Command::OpenImage))
        {
            context.status.clear();
            return vec![ReaderEffect::CancelImage];
        }
        match *mode {
            ImageMode::Selecting(selection) => self.selection_key(context.mode, selection, input),
            ImageMode::Preview(preview) if input.key == InputKey::Character('e') => {
                let payload = preview.payload.clone();
                *context.mode = ReaderMode::Images(Box::new(ImageMode::Preview(preview)));
                vec![ReaderEffect::Viewer(payload)]
            }
            ImageMode::Preview(_) | ImageMode::Failed(_)
                if input.key == InputKey::Enter || input.command == Some(Command::Follow) =>
            {
                vec![ReaderEffect::CancelImage]
            }
            mode => {
                *context.mode = ReaderMode::Images(Box::new(mode));
                Vec::new()
            }
        }
    }

    fn selection_key(
        &mut self,
        mode: &mut ReaderMode,
        mut selection: ImageSelection,
        input: ModeInput,
    ) -> Vec<ReaderEffect> {
        let count = selection.targets.len();
        if count == 0 {
            return Vec::new();
        }
        match input.command {
            Some(Command::ScrollDown | Command::PageDown) => {
                selection.selected = (selection.selected + 1) % count
            }
            Some(Command::ScrollUp | Command::PageUp) => {
                selection.selected = if selection.selected == 0 {
                    count - 1
                } else {
                    selection.selected - 1
                }
            }
            _ => {}
        }
        if (input.key == InputKey::Enter || input.command == Some(Command::Follow))
            && let Some(target) = selection.targets.get(selection.selected).cloned()
        {
            let job = self.load(target);
            *mode = ReaderMode::Images(Box::new(ImageMode::Loading {
                request_id: job.request_id,
            }));
            return vec![ReaderEffect::Image(job)];
        }
        *mode = ReaderMode::Images(Box::new(ImageMode::Selecting(selection)));
        Vec::new()
    }

    fn load(&mut self, target: ImageTarget) -> ImageJob {
        ImageJob {
            request_id: self.requests.next_wrapping(),
            target,
        }
    }

    pub(in crate::app) fn completed(mode: &mut ReaderMode, result: ImageResult) {
        let ReaderMode::Images(image_mode) = mode else {
            return;
        };
        if !matches!(image_mode.as_ref(), ImageMode::Loading { request_id } if *request_id == result.request_id)
        {
            return;
        }
        **image_mode = match result.result {
            Ok(payload) => ImageMode::Preview(ImagePreview {
                generation: result.generation,
                payload,
            }),
            Err(error) => ImageMode::Failed(error),
        };
    }
}

#[derive(Clone, Debug)]
pub struct ImageSelection {
    pub targets: Vec<ImageTarget>,
    pub selected: usize,
}

#[derive(Clone, Debug)]
pub struct ImagePreview {
    pub generation: u64,
    pub payload: Arc<ImagePayload>,
}

#[derive(Clone, Debug)]
pub enum ImageMode {
    Selecting(ImageSelection),
    Loading { request_id: u64 },
    Preview(ImagePreview),
    Failed(String),
}
