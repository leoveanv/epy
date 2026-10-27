#[path = "support/epub.rs"]
mod epub_support;
#[path = "support/pending_web.rs"]
mod pending_web;

use epub_support::{AssetInput, EpubBuilder};
use ers::book::BookLocation;
use ers::images::{ImageJob, ImageResult, ImageTarget, ImageWorker};
use image::ImageFormat;
use pending_web::PendingWeb;
#[path = "support/images.rs"]
mod image_support;
use image_support::ImageBuilder;
#[path = "support/svg.rs"]
mod svg_support;
use std::time::{Duration, Instant};
use svg_support::SvgBuilder;

struct ImageFixture {
    _directory: tempfile::TempDir,
    location: BookLocation,
}

impl ImageFixture {
    fn new() -> Self {
        Self::with_dimensions(4, 3)
    }
    fn with_dimensions(width: u32, height: u32) -> Self {
        let mut builder = ImageBuilder::new();
        builder.width = width;
        builder.height = height;
        Self::with_bytes(builder.bytes())
    }
    fn with_bytes(bytes: Vec<u8>) -> Self {
        Self::with_assets(bytes, Vec::new())
    }
    fn with_assets(bytes: Vec<u8>, assets: Vec<AssetInput>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut builder = EpubBuilder::new("3.0");
        builder
            .assets
            .push(AssetInput::new("preview.png", "image/png", bytes));
        builder.assets.extend(assets);
        let location = BookLocation::local(builder.build(directory.path()));
        Self {
            _directory: directory,
            location,
        }
    }
    fn job(&self, request_id: u64) -> ImageJob {
        ImageJob {
            request_id,
            target: ImageTarget {
                content_id: "OPS/first chapter.xhtml".into(),
                path: "preview.png".into(),
                row_offset: 0,
            },
        }
    }
}

fn completed(worker: &ImageWorker) -> ImageResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = worker.take_result() {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "The image worker did not complete"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn background_images_preserve_the_asset_bytes() {
    let fixture = ImageFixture::new();
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(fixture.location.clone(), fixture.job(1));
    let payload = completed(&worker).result.unwrap();
    assert_eq!(payload.name, "preview.png");
    assert_eq!(
        image::guess_format(&payload.bytes).unwrap(),
        ImageFormat::Png
    );
    assert_eq!(payload.decoded.as_ref().unwrap().pixels.width(), 4);
}

#[test]
fn background_svg_images_render_into_native_pixels() {
    let fixture = ImageFixture::with_bytes(SvgBuilder::new().bytes());
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(fixture.location.clone(), fixture.job(1));
    let payload = completed(&worker).result.unwrap();
    let decoded = payload.decoded.as_ref().unwrap();
    assert_eq!(
        decoded.pixels.to_rgba8().get_pixel(4, 3).0,
        [255, 0, 0, 255]
    );
}

#[test]
fn svg_images_resolve_nested_assets_inside_their_ebook() {
    let mut svg = SvgBuilder::new();
    svg.body = "<image href='parts/pixel.png' width='8' height='6'/>".into();
    let fixture = ImageFixture::with_assets(
        svg.bytes(),
        vec![AssetInput::new(
            "parts/pixel.png",
            "image/png",
            ImageBuilder::new().bytes(),
        )],
    );
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(fixture.location.clone(), fixture.job(1));
    let payload = completed(&worker).result.unwrap();
    let decoded = payload.decoded.as_ref().unwrap();
    assert_eq!(
        decoded.pixels.to_rgba8().get_pixel(4, 3).0,
        [210, 20, 40, 255]
    );
}

#[test]
#[cfg(target_os = "macos")]
fn background_svg_text_retains_its_glyph_pixels() {
    let mut svg = SvgBuilder::new();
    svg.width = 320;
    svg.height = 60;
    svg.body = "<text x='2' y='40' font-size='36' fill='red'>Native SVG text</text>".into();
    let fixture = ImageFixture::with_bytes(svg.bytes());
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(fixture.location.clone(), fixture.job(1));
    let payload = completed(&worker).result.unwrap();
    let decoded = payload.decoded.as_ref().unwrap();
    assert!(
        decoded
            .pixels
            .to_rgba8()
            .pixels()
            .filter(|pixel| pixel.0[3] > 0)
            .count()
            > 100
    );
}

#[test]
fn svg_heap_failures_leave_the_image_worker_available() {
    let svg = ImageFixture::with_bytes(SvgBuilder::new().bytes());
    let png = ImageFixture::new();
    let mut options = ers::images::ImageWorkerOptions::new(env!("CARGO_BIN_EXE_ers").into());
    options.svg_heap_bytes_max = 1024;
    let worker = ImageWorker::start(options);
    worker.request(svg.location.clone(), svg.job(1));
    let payload = completed(&worker).result.unwrap();
    assert!(payload.decoded.as_ref().unwrap_err().contains("heap limit"));
    worker.request(png.location.clone(), png.job(2));
    let payload = completed(&worker).result.unwrap();
    assert_eq!(
        payload
            .decoded
            .as_ref()
            .unwrap()
            .pixels
            .to_rgba8()
            .get_pixel(0, 0)
            .0,
        [210, 20, 40, 255]
    );
}

#[test]
fn oversized_svg_pattern_allocations_do_not_kill_the_reader() {
    let mut builder = SvgBuilder::new();
    builder.body = "<defs><pattern id='huge' patternUnits='userSpaceOnUse' width='16000' height='16000'><rect width='8' height='6' fill='red'/></pattern></defs><rect width='8' height='6' fill='url(#huge)'/>".into();
    let fixture = ImageFixture::with_bytes(builder.bytes());
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(fixture.location.clone(), fixture.job(1));
    let payload = completed(&worker).result.unwrap();
    assert!(payload.decoded.as_ref().unwrap_err().contains("heap limit"));
}

#[test]
#[cfg(unix)]
fn stalled_svg_helpers_obey_the_deadline_while_receiving_input() {
    use std::os::unix::fs::PermissionsExt;
    let mut svg = SvgBuilder::new();
    svg.body = format!("<metadata>{}</metadata>{}", "x".repeat(65_536), svg.body);
    let fixture = ImageFixture::with_bytes(svg.bytes());
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("stalled-decoder");
    std::fs::write(&program, "#!/bin/sh\nexec /bin/sleep 2\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut options = ers::images::ImageWorkerOptions::new(program);
    options.svg_timeout = Duration::from_millis(50);
    let worker = ImageWorker::start(options);
    let started = Instant::now();
    worker.request(fixture.location.clone(), fixture.job(1));
    let payload = completed(&worker).result.unwrap();
    assert!(payload.decoded.as_ref().unwrap_err().contains("deadline"));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "The decoder deadline did not cover a blocked input pipe"
    );
}

#[test]
#[cfg(target_os = "macos")]
fn svg_helpers_retire_when_the_parent_lease_closes() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut svg = SvgBuilder::new();
    svg.body = format!("<text y='5'>{}</text>", "Lease shutdown ".repeat(1000));
    let bytes = svg.bytes();
    let assets = std::collections::BTreeMap::new();
    let request = ers::images::SvgRequest::new(&bytes, &assets);
    let mut child = Command::new(env!("CARGO_BIN_EXE_ers"))
        .args(["--internal-svg-decoder", "268435456"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lease = child.stdin.take().unwrap();
    serde_json::to_writer(&mut lease, &request).unwrap();
    lease.flush().unwrap();
    drop(lease);
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("The SVG helper survived its parent lease");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(status.code(), Some(1));
}

#[test]
fn invalid_preview_data_remains_available_for_error_reporting() {
    let fixture = ImageFixture::with_bytes(b"Bad image".to_vec());
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(fixture.location.clone(), fixture.job(1));
    let payload = completed(&worker).result.unwrap();
    assert!(payload.decoded.as_ref().err().unwrap().contains("image"));
    assert_eq!(payload.bytes, b"Bad image");
}

#[test]
fn identical_asset_names_in_different_books_do_not_share_pixels() {
    let first = ImageFixture::new();
    let second = ImageFixture::with_dimensions(7, 5);
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(first.location.clone(), first.job(1));
    completed(&worker).result.unwrap();
    worker.request(second.location.clone(), second.job(1));
    let payload = completed(&worker).result.unwrap();
    assert_eq!(
        (
            payload.decoded.as_ref().unwrap().pixels.width(),
            payload.decoded.as_ref().unwrap().pixels.height()
        ),
        (7, 5)
    );
}

#[test]
fn newer_image_requests_cancel_pending_http_loading() {
    let remote = PendingWeb::start();
    let fixture = ImageFixture::new();
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(
        BookLocation::parse(&remote.location).unwrap(),
        fixture.job(1),
    );
    remote
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let generation = worker.request(fixture.location.clone(), fixture.job(2));
    let result = completed(&worker);
    assert_eq!((result.generation, result.request_id), (generation, 2));
    assert!(result.result.is_ok());
    remote.respond();
    assert!(worker.take_result().is_none());
}

#[test]
fn canceling_an_image_discards_its_queued_result() {
    let fixture = ImageFixture::new();
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(fixture.location.clone(), fixture.job(1));
    std::thread::sleep(Duration::from_millis(50));
    worker.cancel();
    assert!(worker.take_result().is_none());
}

#[test]
fn stopping_pending_images_does_not_wait_for_http() {
    let remote = PendingWeb::start();
    let fixture = ImageFixture::new();
    let worker = ImageWorker::start(ers::images::ImageWorkerOptions::new(
        env!("CARGO_BIN_EXE_ers").into(),
    ));
    worker.request(
        BookLocation::parse(&remote.location).unwrap(),
        fixture.job(1),
    );
    remote
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let started = Instant::now();
    drop(worker);
    assert!(started.elapsed() < Duration::from_millis(100));
}
