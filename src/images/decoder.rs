use std::io::Cursor;

use crate::cancellation::Cancellation;
use crate::{Error, Result};
use image::{DynamicImage, ImageFormat, ImageReader};

#[derive(Clone, Copy, Debug)]
pub struct ImageLimits {
    pub encoded_bytes_max: usize,
    pub pixels_max: u64,
    pub dimension_px_max: u32,
    pub decoded_bytes_max: u64,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            encoded_bytes_max: 32 * 1024 * 1024,
            pixels_max: 16 * 1024 * 1024,
            dimension_px_max: 16_384,
            decoded_bytes_max: 96 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageSourceFormat {
    Raster(ImageFormat),
    Svg,
}

#[derive(Debug)]
pub struct DecodedImage {
    pub pixels: DynamicImage,
    pub format: ImageSourceFormat,
}

pub fn decode_image(
    bytes: &[u8],
    limits: ImageLimits,
    cancellation: &Cancellation,
) -> Result<DecodedImage> {
    cancellation.check()?;
    if bytes.len() > limits.encoded_bytes_max {
        return Err(Error::Compatibility(
            "The image exceeds its encoded size limit.".into(),
        ));
    }
    let format = image::guess_format(bytes).map_err(image_error)?;
    let (width, height) = image_reader(bytes, format, limits)
        .into_dimensions()
        .map_err(image_error)?;
    if u64::from(width) * u64::from(height) > limits.pixels_max {
        return Err(Error::Compatibility(
            "The image exceeds its decoded pixel limit.".into(),
        ));
    }
    cancellation.check()?;
    let pixels = image_reader(bytes, format, limits)
        .decode()
        .map_err(image_error)?;
    cancellation.check()?;
    Ok(DecodedImage {
        pixels,
        format: ImageSourceFormat::Raster(format),
    })
}

fn image_reader(
    bytes: &[u8],
    format: ImageFormat,
    limits: ImageLimits,
) -> ImageReader<Cursor<&[u8]>> {
    let mut allocation = image::Limits::default();
    allocation.max_image_width = Some(limits.dimension_px_max);
    allocation.max_image_height = Some(limits.dimension_px_max);
    allocation.max_alloc = Some(limits.decoded_bytes_max);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(allocation);
    reader
}

fn image_error(error: image::ImageError) -> Error {
    Error::Compatibility(format!("Cannot preview this image: {error}"))
}
