use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use std::io::Cursor;

pub struct ImageBuilder {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
}

impl ImageBuilder {
    pub fn new() -> Self {
        Self {
            width: 4,
            height: 3,
            format: ImageFormat::Png,
        }
    }
    pub fn bytes(&self) -> Vec<u8> {
        let pixels = ImageBuffer::from_pixel(self.width, self.height, Rgba([210, 20, 40, 255]));
        let image = DynamicImage::ImageRgba8(pixels);
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, self.format).unwrap();
        output.into_inner()
    }
}
