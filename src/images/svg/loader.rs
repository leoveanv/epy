use super::process::decode_isolated;
use super::validation::{svg_error, validate_svg};
use super::{ImageWorkerOptions, SvgDecodeOptions};
use crate::Result;
use crate::book::{Book, BookKind};
use crate::cancellation::Cancellation;
use crate::images::{DecodedImage, ImageLimits, decode_image};
use std::sync::Arc;

pub(in crate::images) struct ImageDecoder {
    options: ImageWorkerOptions,
}

impl ImageDecoder {
    pub fn new(options: ImageWorkerOptions) -> Self {
        Self { options }
    }

    pub fn decode(
        &mut self,
        book: &mut Book,
        content_id: &str,
        bytes: &[u8],
        cancellation: &Cancellation,
    ) -> Result<DecodedImage> {
        if image::guess_format(bytes).is_ok() {
            return decode_image(bytes, ImageLimits::default(), cancellation);
        }
        let mut options = SvgDecodeOptions {
            cancellation: cancellation.clone(),
            ..SvgDecodeOptions::default()
        };
        let source = validate_svg(bytes, &options.limits, cancellation)?;
        if !source.image_refs.is_empty() && book.kind() == BookKind::Web {
            return Err(svg_error("network image references are disabled"));
        }
        let mut encoded_bytes = 0_usize;
        for href in source.image_refs {
            cancellation.check()?;
            let resource = book.resolve_resource(content_id, &href)?;
            let asset = book.asset(&resource.content_id)?;
            encoded_bytes = encoded_bytes.saturating_add(asset.bytes.len());
            if encoded_bytes as u64 > options.limits.embedded_encoded_bytes_max {
                return Err(svg_error("embedded resource size limit exceeded"));
            }
            options.images.insert(href, Arc::new(asset.bytes));
        }
        decode_isolated(bytes, &options.images, &self.options, cancellation)
    }
}
