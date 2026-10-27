use super::validation::{svg_error, validate_svg};
use super::{SvgDecodeOptions, SvgLimits, decode_svg};
use crate::cancellation::Cancellation;
use crate::images::{DecodedImage, ImageSourceFormat};
use crate::{Error, Result};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ImageWorkerOptions {
    pub svg_program: PathBuf,
    pub svg_heap_bytes_max: usize,
    pub svg_timeout: Duration,
}

impl ImageWorkerOptions {
    pub fn new(svg_program: PathBuf) -> Self {
        Self {
            svg_program,
            svg_heap_bytes_max: 256 * 1024 * 1024,
            svg_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SvgResource<'a> {
    href: Cow<'a, str>,
    bytes: Cow<'a, [u8]>,
}

#[derive(Serialize, Deserialize)]
pub struct SvgRequest<'a> {
    source: Cow<'a, [u8]>,
    images: Vec<SvgResource<'a>>,
}

impl<'a> SvgRequest<'a> {
    pub fn new(source: &'a [u8], images: &'a BTreeMap<String, Arc<Vec<u8>>>) -> Self {
        Self {
            source: Cow::Borrowed(source),
            images: images
                .iter()
                .map(|(href, bytes)| SvgResource {
                    href: Cow::Borrowed(href),
                    bytes: Cow::Borrowed(bytes),
                })
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SvgReply {
    Image { width: u32, height: u32 },
    Failed { message: String },
}

pub(super) fn decode_isolated(
    bytes: &[u8],
    images: &BTreeMap<String, Arc<Vec<u8>>>,
    options: &ImageWorkerOptions,
    cancellation: &Cancellation,
) -> Result<DecodedImage> {
    let deadline = Instant::now()
        .checked_add(options.svg_timeout)
        .ok_or_else(|| svg_error("decoder deadline is out of range"))?;
    let mut child = SvgChild::start(options, cancellation, deadline)?;
    child.run(&SvgRequest::new(bytes, images), cancellation, deadline)?;
    child.output.seek(SeekFrom::Start(0))?;
    read_pixels(child.output.try_clone()?)
}

struct CancelWriter<'a> {
    stdin: ChildStdin,
    cancellation: &'a Cancellation,
    deadline: Instant,
}

impl Write for CancelWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.cancellation.is_canceled() {
            return Err(std::io::ErrorKind::ConnectionAborted.into());
        }
        if Instant::now() >= self.deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stdin.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stdin.flush()
    }
}

struct SvgChild<'a> {
    process: Child,
    lease: Option<BufWriter<CancelWriter<'a>>>,
    output: File,
}

impl<'a> SvgChild<'a> {
    fn start(
        options: &ImageWorkerOptions,
        cancellation: &'a Cancellation,
        deadline: Instant,
    ) -> Result<Self> {
        cancellation.check()?;
        let output = tempfile::tempfile()?;
        let mut process = Command::new(&options.svg_program)
            .arg("--internal-svg-decoder")
            .arg(options.svg_heap_bytes_max.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::from(output.try_clone()?))
            .stderr(Stdio::null())
            .spawn()?;
        let Some(stdin) = process.stdin.take() else {
            let _ = process.kill();
            let _ = process.wait();
            return Err(svg_error("the decoder input pipe is unavailable"));
        };
        Ok(Self {
            process,
            output,
            lease: Some(BufWriter::new(CancelWriter {
                stdin,
                cancellation,
                deadline,
            })),
        })
    }

    fn run(
        &mut self,
        request: &SvgRequest<'_>,
        cancellation: &Cancellation,
        deadline: Instant,
    ) -> Result<()> {
        let mut lease = self
            .lease
            .take()
            .ok_or_else(|| svg_error("the decoder lease is unavailable"))?;
        std::thread::scope(|scope| {
            let writer = scope.spawn(move || {
                let result = send_request(&mut lease, request);
                SentRequest {
                    _lease: if result.is_ok() { Some(lease) } else { None },
                    result,
                }
            });
            let result = self.wait(cancellation, deadline);
            let _ = self.process.kill();
            let _ = self.process.wait();
            let sent = writer
                .join()
                .map_err(|_| svg_error("the decoder input writer failed"))?;
            result?;
            sent.result
        })
    }

    fn wait(&mut self, cancellation: &Cancellation, deadline: Instant) -> Result<()> {
        loop {
            cancellation.check()?;
            if let Some(status) = self.process.try_wait()? {
                return if status.success() {
                    Ok(())
                } else if status.code() == Some(86) {
                    Err(svg_error("decoder heap limit exceeded"))
                } else {
                    Err(svg_error("the isolated decoder failed"))
                };
            }
            if Instant::now() >= deadline {
                return Err(svg_error("decoder deadline exceeded"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for SvgChild<'_> {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

struct SentRequest<'a> {
    _lease: Option<BufWriter<CancelWriter<'a>>>,
    result: Result<()>,
}

fn send_request(lease: &mut BufWriter<CancelWriter<'_>>, request: &SvgRequest<'_>) -> Result<()> {
    let result = serde_json::to_writer(&mut *lease, request)
        .map_err(Error::from)
        .and_then(|()| lease.flush().map_err(Error::from));
    lease.get_ref().cancellation.check()?;
    if result.is_err() && Instant::now() >= lease.get_ref().deadline {
        return Err(svg_error("decoder deadline exceeded"));
    }
    result
}

fn read_pixels(output: File) -> Result<DecodedImage> {
    let length = output.metadata()?.len();
    let mut input = BufReader::new(output);
    let mut header = String::new();
    input.by_ref().take(4096).read_line(&mut header)?;
    let reply = serde_json::from_str(&header)?;
    match reply {
        SvgReply::Failed { message } => Err(Error::Compatibility(message)),
        SvgReply::Image { width, height } => {
            let pixels = u64::from(width) * u64::from(height);
            if width == 0
                || height == 0
                || pixels > SvgLimits::default().render_pixels_max
                || length != header.len() as u64 + pixels * 4
            {
                return Err(svg_error("invalid decoder output bounds"));
            }
            let mut bytes = vec![0; pixels as usize * 4];
            input.read_exact(&mut bytes)?;
            let pixels = RgbaImage::from_raw(width, height, bytes)
                .ok_or_else(|| svg_error("invalid decoder pixels"))?;
            Ok(DecodedImage {
                pixels: DynamicImage::ImageRgba8(pixels),
                format: ImageSourceFormat::Svg,
            })
        }
    }
}

pub fn run_svg_helper() -> Result<()> {
    let request = read_request()?;
    watch_parent();
    write_reply(render_request(request))
}

fn read_request() -> Result<SvgRequest<'static>> {
    let stdin = std::io::stdin();
    let mut input = serde_json::Deserializer::from_reader(stdin.lock().take(96 * 1024 * 1024));
    Ok(SvgRequest::deserialize(&mut input)?)
}

fn write_reply(decoded: Result<DecodedImage>) -> Result<()> {
    let mut output = BufWriter::new(std::io::stdout().lock());
    match decoded {
        Ok(image) => {
            serde_json::to_writer(
                &mut output,
                &SvgReply::Image {
                    width: image.pixels.width(),
                    height: image.pixels.height(),
                },
            )?;
            output.write_all(b"\n")?;
            output.write_all(image.pixels.as_bytes())?;
        }
        Err(error) => {
            serde_json::to_writer(
                &mut output,
                &SvgReply::Failed {
                    message: error.to_string(),
                },
            )?;
            output.write_all(b"\n")?;
        }
    }
    output.flush()?;
    Ok(())
}

fn render_request(request: SvgRequest<'_>) -> Result<DecodedImage> {
    let mut options = SvgDecodeOptions::default();
    let source = validate_svg(&request.source, &options.limits, &options.cancellation)?;
    if request.images.len() > options.limits.resources_max {
        return Err(svg_error("image resource count limit exceeded"));
    }
    if source.needs_fonts {
        let mut fonts = resvg::usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        options.fonts = Arc::new(fonts);
    }
    options.images = request
        .images
        .into_iter()
        .map(|image| (image.href.into_owned(), Arc::new(image.bytes.into_owned())))
        .collect();
    decode_svg(&request.source, &options)
}

fn watch_parent() {
    // #NOTE: The parent keeps stdin open as a lease; EOF also retires the helper after abrupt parent death.
    std::thread::spawn(|| {
        let mut byte = [0];
        let _ = std::io::stdin().read(&mut byte);
        std::process::exit(1);
    });
}
