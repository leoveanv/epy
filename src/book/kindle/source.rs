use crate::book::ResourceLimits;
use crate::cancellation::Cancellation;
use crate::{Error, Result};
use boko::{ByteSource, FileSource};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

pub(super) struct LimitedSource {
    source: FileSource,
    limits: ResourceLimits,
    cancellation: Cancellation,
}

impl LimitedSource {
    pub fn from_file(
        file: std::fs::File,
        limits: ResourceLimits,
        cancellation: Cancellation,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            source: FileSource::new(file)?,
            limits,
            cancellation,
        }))
    }

    pub fn open(
        path: &Path,
        limits: ResourceLimits,
        cancellation: Cancellation,
    ) -> Result<Arc<Self>> {
        Self::from_file(std::fs::File::open(path)?, limits, cancellation)
    }
}

impl ByteSource for LimitedSource {
    fn len(&self) -> u64 {
        self.source.len()
    }

    fn read_at_into(&self, offset: u64, bytes: &mut [u8]) -> io::Result<usize> {
        if self.cancellation.is_canceled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Kindle loading canceled",
            ));
        }
        if bytes.len() as u64 > self.limits.resource_bytes_max
            || offset
                .checked_add(bytes.len() as u64)
                .is_none_or(|end| end > self.len())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Kindle record exceeds its bounds",
            ));
        }
        self.source.read_at_into(offset, bytes)
    }

    fn read_at(&self, offset: u64, length: usize) -> io::Result<Vec<u8>> {
        if length as u64 > self.limits.resource_bytes_max
            || offset
                .checked_add(length as u64)
                .is_none_or(|end| end > self.len())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Kindle record exceeds its bounds",
            ));
        }
        let mut bytes = vec![0; length];
        if self.read_at_into(offset, &mut bytes)? != length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Kindle record is truncated",
            ));
        }
        Ok(bytes)
    }
}

pub(super) struct RecordSource {
    pub source: Arc<LimitedSource>,
    offsets: Vec<u64>,
    pub name: Vec<u8>,
}

impl RecordSource {
    pub fn open(source: Arc<LimitedSource>, limits: ResourceLimits) -> Result<Self> {
        if source.len() > limits.archive_bytes_max || source.len() < 78 {
            return Err(Error::Compatibility(
                "The Kindle file exceeds its size bounds.".into(),
            ));
        }
        let header = source.read_at(0, 78)?;
        if !matches!(&header[60..68], b"BOOKMOBI" | b"TEXtREAd") {
            return Err(Error::Compatibility(
                "The file is not a supported Kindle/PalmDOC book.".into(),
            ));
        }
        let count = u16::from_be_bytes([header[76], header[77]]) as usize;
        if count < 2 || count > limits.archive_entries_max {
            return Err(Error::Compatibility(
                "The Kindle record count is invalid.".into(),
            ));
        }
        let table = source.read_at(78, count * 8)?;
        let mut offsets = table
            .chunks_exact(8)
            .map(|entry| {
                u32::from_be_bytes(
                    entry[..4]
                        .try_into()
                        .expect("A record offset has four bytes"),
                ) as u64
            })
            .collect::<Vec<_>>();
        if offsets[0] < (78 + count * 8) as u64
            || offsets.windows(2).any(|pair| pair[0] > pair[1])
            || offsets.last().is_some_and(|offset| *offset > source.len())
        {
            return Err(Error::Compatibility(
                "The Kindle record offsets are invalid.".into(),
            ));
        }
        offsets.push(source.len());
        Ok(Self {
            source,
            offsets,
            name: header[..32].to_vec(),
        })
    }

    pub fn prefix(&self) -> Result<Vec<u8>> {
        Ok(self.source.read_at(0, self.offsets[0] as usize)?)
    }

    pub fn count(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn copy(&self, index: usize, output: &mut impl Write) -> Result<u64> {
        let range = self
            .offsets
            .get(index..)
            .and_then(|tail| tail.get(..2))
            .ok_or_else(|| {
                Error::Compatibility("The Kindle record is outside its table.".into())
            })?;
        let mut buffer = [0u8; 16 * 1024];
        let mut offset = range[0];
        while offset < range[1] {
            let length = (range[1] - offset)
                .min(buffer.len() as u64)
                .min(self.source.limits.resource_bytes_max) as usize;
            let count = self.source.read_at_into(offset, &mut buffer[..length])?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Kindle record is truncated",
                )
                .into());
            }
            output.write_all(&buffer[..count])?;
            offset += count as u64;
        }
        Ok(range[1] - range[0])
    }

    pub fn read(&self, index: usize) -> Result<Vec<u8>> {
        let range = self
            .offsets
            .get(index..)
            .and_then(|tail| tail.get(..2))
            .ok_or_else(|| {
                Error::Compatibility("The Kindle record is outside its table.".into())
            })?;
        Ok(self
            .source
            .read_at(range[0], (range[1] - range[0]) as usize)?)
    }
}
