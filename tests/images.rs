#[path = "support/images.rs"]
mod image_support;
use image_support::ImageBuilder;
use std::sync::Arc;

use ers::cancellation::Cancellation;
use ers::document::RenderedDocument;
use ers::document::text::parse_document;
use ers::images::{
    ImageEncoding, ImageLimits, ImageProtocol, ImageSourceFormat, decode_image, prepare_image,
    visible_images,
};
use ers::reading::{DocumentRow, ReadingWindow, WindowExtent};
use image::ImageFormat;

fn document(index: usize, body: &str) -> Arc<RenderedDocument> {
    Arc::new(RenderedDocument::new(
        index,
        parse_document(&format!("OPS/chapter-{index}.xhtml"), body).unwrap(),
        40,
    ))
}

#[test]
fn prepared_halfblock_images_preserve_the_pixel_color() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
    let decoded = decode_image(
        &ImageBuilder::new().bytes(),
        ImageLimits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let encoding = ImageEncoding {
        columns: 20,
        rows: 10,
        cell_width_px: 8,
        cell_height_px: 16,
        protocol: ImageProtocol::Halfblocks,
    };
    let prepared = prepare_image(&decoded.pixels, encoding).unwrap();
    let area = Rect::new(0, 0, 20, 10);
    let mut buffer = Buffer::empty(area);
    ratatui_image::Image::new(&prepared.protocol).render(area, &mut buffer);
    assert_eq!(buffer[(0, 0)].fg, Color::Rgb(210, 20, 40));
}

#[test]
fn prepared_images_fit_the_available_character_cells() {
    let decoded = decode_image(
        &ImageBuilder::new().bytes(),
        ImageLimits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let encoding = ImageEncoding {
        columns: 20,
        rows: 10,
        cell_width_px: 8,
        cell_height_px: 16,
        protocol: ImageProtocol::Halfblocks,
    };
    let prepared = prepare_image(&decoded.pixels, encoding).unwrap();
    assert_eq!(prepared.protocol.size(), ratatui::layout::Size::new(20, 8));
}

#[test]
fn kitty_previews_fit_the_placeholder_coordinate_range() {
    let decoded = decode_image(
        &ImageBuilder::new().bytes(),
        ImageLimits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let encoding = ImageEncoding {
        columns: 400,
        rows: 400,
        cell_width_px: 1,
        cell_height_px: 1,
        protocol: ImageProtocol::Kitty {
            image_id: 42,
            in_tmux: false,
        },
    };
    let prepared = prepare_image(&decoded.pixels, encoding).unwrap();
    assert_eq!(
        prepared.protocol.size(),
        ratatui::layout::Size::new(296, 222)
    );
}

#[test]
fn tmux_kitty_images_use_a_private_local_file_instead_of_streaming_pixels() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::PermissionsExt;

    let decoded = decode_image(
        &ImageBuilder::new().bytes(),
        ImageLimits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let prepared = prepare_image(
        &decoded.pixels,
        ImageEncoding {
            columns: 20,
            rows: 10,
            cell_width_px: 8,
            cell_height_px: 16,
            protocol: ImageProtocol::Kitty {
                image_id: 42,
                in_tmux: true,
            },
        },
    )
    .unwrap();
    let area = Rect::new(0, 0, 20, 10);
    let mut buffer = Buffer::empty(area);
    ratatui_image::Image::new(&prepared.protocol).render(area, &mut buffer);
    let symbol = buffer[(0, 0)].symbol();
    assert!(symbol.starts_with("\u{1b}Ptmux;\u{1b}\u{1b}_G"));
    let (_, payload) = symbol.split_once("t=f;").expect("No file transfer");
    let encoded_path = payload.split('\u{1b}').next().unwrap();
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(
        STANDARD.decode(encoded_path).unwrap(),
    ));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let pixels = image::load_from_memory(&std::fs::read(&path).unwrap())
        .unwrap()
        .to_rgb8();
    assert_eq!(pixels.get_pixel(0, 0).0, [210, 20, 40]);
    drop(prepared);
    assert!(!path.exists());
}

#[test]
fn kitty_cleanup_targets_only_the_owned_image() {
    let decoded = decode_image(
        &ImageBuilder::new().bytes(),
        ImageLimits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let encoding = ImageEncoding {
        columns: 20,
        rows: 10,
        cell_width_px: 8,
        cell_height_px: 16,
        protocol: ImageProtocol::Kitty {
            image_id: 42,
            in_tmux: false,
        },
    };
    let prepared = prepare_image(&decoded.pixels, encoding).unwrap();
    assert_eq!(
        prepared.cleanup_sequence(),
        Some("\u{1b}_Ga=d,d=I,i=42,q=2\u{1b}\\".into())
    );
}

#[test]
fn image_decoding_retains_the_actual_format_and_dimensions() {
    let builder = ImageBuilder::new();
    let decoded = decode_image(
        &builder.bytes(),
        ImageLimits::default(),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(decoded.format, ImageSourceFormat::Raster(ImageFormat::Png));
    assert_eq!(
        (decoded.pixels.width(), decoded.pixels.height()),
        (builder.width, builder.height)
    );
}

#[test]
fn image_pixel_limits_reject_oversized_decoded_images() {
    let builder = ImageBuilder::new();
    let limits = ImageLimits {
        pixels_max: 4,
        ..ImageLimits::default()
    };
    let error = decode_image(&builder.bytes(), limits, &Cancellation::default())
        .err()
        .unwrap();
    assert!(error.to_string().contains("pixel limit"));
}

#[test]
fn image_byte_limits_reject_oversized_sources() {
    let bytes = ImageBuilder::new().bytes();
    let limits = ImageLimits {
        encoded_bytes_max: bytes.len() - 1,
        ..ImageLimits::default()
    };
    let error = decode_image(&bytes, limits, &Cancellation::default())
        .err()
        .unwrap();
    assert!(error.to_string().contains("size limit"));
}

#[test]
fn canceled_image_requests_do_not_decode() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let result = decode_image(
        &ImageBuilder::new().bytes(),
        ImageLimits::default(),
        &cancellation,
    );
    assert!(matches!(result, Err(ers::Error::Canceled)));
}

#[test]
fn invalid_image_data_reports_a_preview_error() {
    let result = decode_image(
        b"Not an image",
        ImageLimits::default(),
        &Cancellation::default(),
    );
    assert!(result.err().unwrap().to_string().contains("image"));
}

#[test]
fn visible_images_preserve_each_chapters_resource_context() {
    let first = document(0, "<p>First</p><img src='first.png'/>");
    let first_rows = first.layout.text_lines.len();
    let second = document(1, "<img src='second.png'/>");
    let window = ReadingWindow::new(
        vec![first, second],
        WindowExtent {
            starts_book: true,
            ends_book: true,
        },
    )
    .unwrap();
    let images = visible_images(
        &window,
        DocumentRow {
            document_index: 0,
            row: 0,
        },
        first_rows + 4,
    );
    assert_eq!(
        images
            .iter()
            .map(|target| target.path.as_str())
            .collect::<Vec<_>>(),
        ["first.png", "second.png"]
    );
    assert_eq!(
        images
            .iter()
            .map(|target| target.content_id.as_str())
            .collect::<Vec<_>>(),
        ["OPS/chapter-0.xhtml", "OPS/chapter-1.xhtml"]
    );
}

#[test]
fn offscreen_images_are_not_selected() {
    let document = document(0, "<img src='first.png'/><p>Following</p>");
    let image_row = *document.layout.image_maps.keys().next().unwrap();
    let window = ReadingWindow::single(
        document,
        WindowExtent {
            starts_book: true,
            ends_book: true,
        },
    );
    assert!(
        visible_images(
            &window,
            DocumentRow {
                document_index: 0,
                row: image_row + 1
            },
            1
        )
        .is_empty()
    );
}
