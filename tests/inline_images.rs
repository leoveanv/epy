use ers::images::{ImageEncoding, ImageProtocol, PreparedInlineImage, prepare_image};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
use ratatui_image::sliced::{SignedPosition, SlicedImage};

struct InlineImageBuilder {
    protocol: ImageProtocol,
}

impl InlineImageBuilder {
    fn new() -> Self {
        Self {
            protocol: ImageProtocol::Halfblocks,
        }
    }

    fn build(&self) -> PreparedInlineImage {
        let pixels = RgbaImage::from_fn(2, 8, |_, y| {
            if y < 4 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        prepare_image(
            &DynamicImage::ImageRgba8(pixels),
            ImageEncoding {
                columns: 2,
                rows: 4,
                cell_width_px: 1,
                cell_height_px: 2,
                protocol: self.protocol,
            },
        )
        .unwrap()
        .into_sliced()
        .unwrap()
    }
}

#[test]
fn partial_halfblock_images_start_at_the_visible_pixel_rows() {
    let image = InlineImageBuilder::new().build();
    let area = Rect::new(0, 0, 2, 2);
    let mut buffer = Buffer::empty(area);
    SlicedImage::new(&image.protocol, SignedPosition::from((0, -2))).render(area, &mut buffer);
    assert_eq!(buffer[(0, 0)].fg, Color::Rgb(0, 0, 255));
    assert_eq!(buffer[(0, 1)].fg, Color::Rgb(0, 0, 255));
}

#[test]
fn inline_images_do_not_paint_outside_their_viewport() {
    let image = InlineImageBuilder::new().build();
    let area = Rect::new(0, 0, 6, 6);
    let mut buffer = Buffer::empty(area);
    buffer.set_string(0, 3, "......", ratatui::style::Style::default());
    SlicedImage::new(&image.protocol, SignedPosition::from((0, 0)))
        .render(Rect::new(2, 2, 2, 1), &mut buffer);
    assert_eq!(buffer[(2, 2)].fg, Color::Rgb(255, 0, 0));
    assert_eq!(buffer[(2, 3)].symbol(), ".");
}

#[test]
fn partially_visible_kitty_images_transmit_before_their_placeholders() {
    let mut builder = InlineImageBuilder::new();
    builder.protocol = ImageProtocol::Kitty {
        image_id: 42,
        in_tmux: false,
    };
    let image = builder.build();
    let area = Rect::new(0, 0, 2, 2);
    let mut buffer = Buffer::empty(area);
    SlicedImage::new(&image.protocol, SignedPosition::from((0, -2))).render(area, &mut buffer);
    let first_cell = buffer[(0, 0)].symbol();
    assert!(first_cell.starts_with("\x1b_G"));
    assert!(first_cell.split(';').next().unwrap().contains("i=42,"));
}

#[test]
fn sliced_tmux_images_keep_their_private_file_until_they_are_dropped() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::unix::ffi::OsStringExt;

    let mut builder = InlineImageBuilder::new();
    builder.protocol = ImageProtocol::Kitty {
        image_id: 42,
        in_tmux: true,
    };
    let image = builder.build();
    let area = Rect::new(0, 0, 2, 2);
    let mut buffer = Buffer::empty(area);
    SlicedImage::new(&image.protocol, SignedPosition::from((0, -2))).render(area, &mut buffer);
    let (_, payload) = buffer[(0, 0)].symbol().split_once("t=f;").unwrap();
    let encoded_path = payload.split('\u{1b}').next().unwrap();
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(
        STANDARD.decode(encoded_path).unwrap(),
    ));
    assert!(path.is_file());
    assert_eq!(
        image.cleanup_sequence().as_deref(),
        Some("\x1bPtmux;\x1b\x1b_Ga=d,d=I,i=42,q=2\x1b\x1b\\\x1b\\")
    );
    drop(image);
    assert!(!path.exists());
}

#[test]
fn sliced_kitty_images_keep_their_owned_cleanup_identity() {
    let mut builder = InlineImageBuilder::new();
    builder.protocol = ImageProtocol::Kitty {
        image_id: 42,
        in_tmux: false,
    };
    let image = builder.build();
    assert_eq!(
        image.cleanup_sequence().as_deref(),
        Some("\x1b_Ga=d,d=I,i=42,q=2\x1b\\")
    );
}
