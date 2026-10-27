#[path = "support/images.rs"]
mod image_support;

use ers::integrations::{ImageViewer, viewer_command};
use image_support::ImageBuilder;
use std::path::Path;

#[test]
fn viewer_commands_preserve_quoted_arguments() {
    let command = viewer_command(
        "viewer --profile 'reading profile'",
        Path::new("/tmp/image name.png"),
    )
    .unwrap();
    assert_eq!(
        command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect::<Vec<_>>(),
        ["--profile", "reading profile", "/tmp/image name.png"]
    );
}

#[test]
fn viewer_commands_do_not_interpret_shell_metacharacters() {
    let command = viewer_command("viewer ';' '$HOME'", Path::new("/tmp/image.png")).unwrap();
    assert_eq!(
        command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect::<Vec<_>>(),
        [";", "$HOME", "/tmp/image.png"]
    );
}

#[cfg(unix)]
struct ViewerFixture {
    _directory: tempfile::TempDir,
    command: String,
    recorded: std::path::PathBuf,
}

#[cfg(unix)]
impl ViewerFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let recorded = directory.path().join("path");
        let helper = directory.path().join("viewer.sh");
        std::fs::write(
            &helper,
            format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", recorded.display()),
        )
        .unwrap();
        Self {
            command: format!("/bin/sh '{}'", helper.display()),
            recorded,
            _directory: directory,
        }
    }
    fn path(&self) -> std::path::PathBuf {
        std::fs::read_to_string(&self.recorded).unwrap().into()
    }
}

#[test]
#[cfg(unix)]
fn external_viewer_files_survive_until_the_reader_session_ends() {
    let fixture = ViewerFixture::new();
    let mut viewer = ImageViewer::default();
    let bytes = ImageBuilder::new().bytes();
    viewer.open(&bytes, &fixture.command).unwrap();
    let path = fixture.path();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(path.extension().unwrap(), "png");
    drop(viewer);
    assert!(!path.exists());
}

#[test]
#[cfg(unix)]
fn rendered_external_images_are_safe_png_pixels() {
    let fixture = ViewerFixture::new();
    let mut viewer = ImageViewer::default();
    let pixels = image::load_from_memory(&ImageBuilder::new().bytes()).unwrap();
    viewer.open_rendered(&pixels, &fixture.command).unwrap();
    let bytes = std::fs::read(fixture.path()).unwrap();
    assert_eq!(
        image::guess_format(&bytes).unwrap(),
        image::ImageFormat::Png
    );
    assert_eq!(
        image::load_from_memory(&bytes).unwrap().to_rgba8(),
        pixels.to_rgba8()
    );
}

#[test]
#[cfg(unix)]
fn repeated_external_images_reuse_the_private_file() {
    let fixture = ViewerFixture::new();
    let mut viewer = ImageViewer::default();
    let bytes = ImageBuilder::new().bytes();
    viewer.open(&bytes, &fixture.command).unwrap();
    let first = fixture.path();
    viewer.open(&bytes, &fixture.command).unwrap();
    assert_eq!(fixture.path(), first);
}

#[test]
#[cfg(unix)]
fn external_viewers_reject_unrecognized_payloads_before_launch() {
    let fixture = ViewerFixture::new();
    let mut viewer = ImageViewer::default();
    assert!(
        viewer
            .open(b"#!/bin/sh\nmalicious command\n", &fixture.command)
            .is_err()
    );
    assert!(!fixture.recorded.exists());
}

#[test]
#[cfg(unix)]
fn external_viewer_failures_are_reported() {
    let mut viewer = ImageViewer::default();
    let result = viewer.open(&ImageBuilder::new().bytes(), "/usr/bin/false");
    assert!(result.unwrap_err().to_string().contains("status"));
}
