mod cost;
mod loader;
mod process;
mod styles;
mod validation;

pub(super) use loader::ImageDecoder;
pub use process::{ImageWorkerOptions, SvgRequest, run_svg_helper};

use super::{DecodedImage, ImageLimits, ImageSourceFormat, decode_image};
use crate::Result;
use crate::cancellation::Cancellation;
use cost::render_scale;
use image::{DynamicImage, ImageFormat, RgbaImage};
use resvg::{tiny_skia, usvg};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use validation::{svg_error, validate_svg};

#[derive(Clone, Copy)]
pub struct SvgLimits {
    pub encoded_bytes_max: usize,
    pub nodes_max: usize,
    pub depth_max: usize,
    pub text_characters_max: usize,
    pub path_bytes_max: usize,
    pub render_pixels_max: u64,
    pub scratch_bytes_max: u64,
    pub work_pixels_max: u64,
    pub resources_max: usize,
    pub embedded_encoded_bytes_max: u64,
    pub embedded_pixels_max: u64,
    pub css_bytes_max: usize,
    pub css_rules_max: usize,
}

impl Default for SvgLimits {
    fn default() -> Self {
        Self {
            encoded_bytes_max: 4 * 1024 * 1024,
            nodes_max: 20_000,
            depth_max: 32,
            text_characters_max: 32_768,
            path_bytes_max: 512 * 1024,
            render_pixels_max: 1024 * 1024,
            scratch_bytes_max: 96 * 1024 * 1024,
            work_pixels_max: 64 * 1024 * 1024,
            resources_max: 32,
            embedded_encoded_bytes_max: 16 * 1024 * 1024,
            embedded_pixels_max: 4 * 1024 * 1024,
            css_bytes_max: 8192,
            css_rules_max: 128,
        }
    }
}

#[derive(Default)]
pub struct SvgDecodeOptions {
    pub limits: SvgLimits,
    pub image_limits: ImageLimits,
    pub cancellation: Cancellation,
    pub fonts: Arc<usvg::fontdb::Database>,
    pub images: BTreeMap<String, Arc<Vec<u8>>>,
}

pub fn decode_svg(bytes: &[u8], options: &SvgDecodeOptions) -> Result<DecodedImage> {
    let text = validate_svg(bytes, &options.limits, &options.cancellation)?;
    let embedded = Arc::new(EmbeddedBudget::new(options));
    let parser = parser_options(options, embedded.clone());
    let tree =
        usvg::Tree::from_str(text.text, &parser).map_err(|error| svg_error(&error.to_string()))?;
    options.cancellation.check()?;
    if embedded.refused.load(Ordering::Relaxed) {
        return Err(svg_error(
            "an embedded image is unsupported or exceeds its resource limit",
        ));
    }
    let scale = render_scale(&tree, options)?;
    let pixels = render_pixels(&tree, scale, &options.cancellation)?;
    Ok(DecodedImage {
        pixels,
        format: ImageSourceFormat::Svg,
    })
}

struct EmbeddedBudget {
    remaining_pixels: AtomicU64,
    remaining_bytes: AtomicU64,
    refused: AtomicBool,
}

impl EmbeddedBudget {
    fn new(options: &SvgDecodeOptions) -> Self {
        Self {
            remaining_pixels: AtomicU64::new(
                options
                    .image_limits
                    .pixels_max
                    .min(options.limits.embedded_pixels_max),
            ),
            remaining_bytes: AtomicU64::new(options.limits.embedded_encoded_bytes_max),
            refused: AtomicBool::new(false),
        }
    }

    fn resolve(
        &self,
        data: Arc<Vec<u8>>,
        limits: ImageLimits,
        cancellation: &Cancellation,
    ) -> Option<usvg::ImageKind> {
        if self
            .remaining_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(data.len() as u64)
            })
            .is_err()
        {
            self.refused.store(true, Ordering::Relaxed);
            return None;
        }
        let result = decode_image(&data, limits, cancellation)
            .ok()
            .and_then(|decoded| {
                let pixels = u64::from(decoded.pixels.width()) * u64::from(decoded.pixels.height());
                self.remaining_pixels
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                        remaining.checked_sub(pixels)
                    })
                    .ok()?;
                match decoded.format {
                    ImageSourceFormat::Raster(ImageFormat::Png) => Some(usvg::ImageKind::PNG(data)),
                    ImageSourceFormat::Raster(ImageFormat::Jpeg) => {
                        Some(usvg::ImageKind::JPEG(data))
                    }
                    ImageSourceFormat::Raster(ImageFormat::Gif) => Some(usvg::ImageKind::GIF(data)),
                    ImageSourceFormat::Raster(ImageFormat::WebP) => {
                        Some(usvg::ImageKind::WEBP(data))
                    }
                    _ => None,
                }
            });
        if result.is_none() {
            self.refused.store(true, Ordering::Relaxed);
        }
        result
    }
}

fn parser_options(
    options: &SvgDecodeOptions,
    embedded: Arc<EmbeddedBudget>,
) -> usvg::Options<'static> {
    let limits = ImageLimits {
        pixels_max: options
            .image_limits
            .pixels_max
            .min(options.limits.embedded_pixels_max),
        ..options.image_limits
    };
    let cancellation = options.cancellation.clone();
    let resource_cancellation = cancellation.clone();
    let resources = options.images.clone();
    let resource_budget = embedded.clone();
    usvg::Options {
        fontdb: options.fonts.clone(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(move |href, _| match resources.get(href) {
                Some(data) => resource_budget.resolve(data.clone(), limits, &resource_cancellation),
                None => {
                    resource_budget.refused.store(true, Ordering::Relaxed);
                    None
                }
            }),
            resolve_data: Box::new(move |_, data, _| embedded.resolve(data, limits, &cancellation)),
        },
        ..usvg::Options::default()
    }
}

fn render_pixels(
    tree: &usvg::Tree,
    scale: f32,
    cancellation: &Cancellation,
) -> Result<DynamicImage> {
    let width = (tree.size().width() * scale).floor().max(1.0) as u32;
    let height = (tree.size().height() * scale).floor().max(1.0) as u32;
    cancellation.check()?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| svg_error("cannot allocate the output canvas"))?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    cancellation.check()?;
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    RgbaImage::from_raw(width, height, rgba)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| svg_error("invalid output geometry"))
}
