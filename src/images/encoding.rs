use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use ratatui::layout::Size;
use ratatui_image::protocol::{Protocol, halfblocks::Halfblocks, kitty::Kitty};
use ratatui_image::sliced::SlicedProtocol;
use std::io::Write;
use tempfile::NamedTempFile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageProtocol {
    Halfblocks,
    Kitty { image_id: u32, in_tmux: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageEncoding {
    pub columns: u16,
    pub rows: u16,
    pub cell_width_px: u16,
    pub cell_height_px: u16,
    pub protocol: ImageProtocol,
}

pub struct PreparedImage {
    pub protocol: Protocol,
    kitty: Option<KittyImage>,
}

pub struct PreparedInlineImage {
    pub protocol: SlicedProtocol,
    kitty: Option<KittyImage>,
}

struct KittyImage {
    id: u32,
    in_tmux: bool,
    _file: Option<NamedTempFile>,
}

impl KittyImage {
    fn cleanup_sequence(&self) -> String {
        let sequence = format!("\x1b_Ga=d,d=I,i={},q=2\x1b\\", self.id);
        if self.in_tmux {
            format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
        } else {
            sequence
        }
    }
}

impl PreparedImage {
    pub fn cleanup_sequence(&self) -> Option<String> {
        self.kitty.as_ref().map(KittyImage::cleanup_sequence)
    }

    pub fn into_sliced(self) -> Result<PreparedInlineImage> {
        let protocol = match self.protocol {
            Protocol::Halfblocks(protocol) => SlicedProtocol::Halfblocks(protocol),
            Protocol::Kitty(protocol) => SlicedProtocol::Kitty(protocol),
            _ => {
                return Err(Error::Compatibility(
                    "The prepared protocol cannot display partial images.".into(),
                ));
            }
        };
        Ok(PreparedInlineImage {
            protocol,
            kitty: self.kitty,
        })
    }
}

impl PreparedInlineImage {
    pub fn cleanup_sequence(&self) -> Option<String> {
        self.kitty.as_ref().map(KittyImage::cleanup_sequence)
    }
}

// #NOTE: Limit preparation to four million pixels even when the terminal reports extreme geometry.
pub fn prepare_image(pixels: &DynamicImage, encoding: ImageEncoding) -> Result<PreparedImage> {
    if encoding.columns == 0
        || encoding.rows == 0
        || encoding.cell_width_px == 0
        || encoding.cell_height_px == 0
    {
        return Err(Error::Compatibility("The image viewport is empty.".into()));
    }
    // #NOTE: Kitty placeholders have a finite diacritic coordinate range; larger images silently lose cells.
    let cells_max = if matches!(encoding.protocol, ImageProtocol::Kitty { .. }) {
        296
    } else {
        u16::MAX
    };
    let width =
        (u32::from(encoding.columns.min(cells_max)) * u32::from(encoding.cell_width_px)).min(2048);
    let height =
        (u32::from(encoding.rows.min(cells_max)) * u32::from(encoding.cell_height_px)).min(2048);
    let resized = pixels.resize(width, height, FilterType::Triangle);
    let size = Size::new(
        resized.width().div_ceil(u32::from(encoding.cell_width_px)) as u16,
        resized
            .height()
            .div_ceil(u32::from(encoding.cell_height_px)) as u16,
    );
    let (protocol, kitty) = match encoding.protocol {
        ImageProtocol::Halfblocks => (
            Protocol::Halfblocks(Halfblocks::new(resized, size).map_err(encoding_error)?),
            None,
        ),
        ImageProtocol::Kitty { image_id, in_tmux } => {
            if image_id == 0 {
                return Err(Error::Compatibility(
                    "A Kitty image requires a nonzero image ID.".into(),
                ));
            }
            let (protocol, file) = if in_tmux {
                let (protocol, file) = prepare_tmux_kitty(&resized, size, image_id)?;
                (protocol, Some(file))
            } else {
                (
                    Kitty::new(resized, size, image_id, false, false).map_err(encoding_error)?,
                    None,
                )
            };
            (
                Protocol::Kitty(protocol),
                Some(KittyImage {
                    id: image_id,
                    in_tmux,
                    _file: file,
                }),
            )
        }
    };
    Ok(PreparedImage { protocol, kitty })
}

fn prepare_tmux_kitty(
    pixels: &DynamicImage,
    size: Size,
    image_id: u32,
) -> Result<(Kitty, NamedTempFile)> {
    let mut file = NamedTempFile::new()?;
    pixels
        .write_to(file.as_file_mut(), ImageFormat::Png)
        .map_err(|error| {
            Error::Compatibility(format!("Cannot prepare the terminal image: {error}"))
        })?;
    file.as_file_mut().flush()?;
    let path = STANDARD.encode(file.path().as_os_str().as_encoded_bytes());
    let sequence =
        format!("\x1bPtmux;\x1b\x1b_Gq=2,i={image_id},a=T,U=1,f=100,t=f;{path}\x1b\x1b\\\x1b\\");
    Ok((Kitty::from_transmission(size, image_id, sequence), file))
}

fn encoding_error(error: ratatui_image::errors::Errors) -> Error {
    Error::Compatibility(format!("Cannot prepare the terminal image: {error}"))
}
