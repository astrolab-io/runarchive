use async_compression::tokio::bufread::DeflateDecoder;
use std::io::{Error as IoError, ErrorKind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::BufReader;
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::io::StreamReader;

use crate::parser;
use crate::reader::Reader;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: Arc<str>,
    pub size: u64,
    pub compressed_size: u64,
    pub compression_method: u16,
    pub offset: u64,
}

pub enum ArchiveReader<R> {
    Stored(R),
    Deflated(DeflateDecoder<BufReader<R>>),
}

impl<R: AsyncRead + Unpin + Send + 'static> AsyncRead for ArchiveReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ArchiveReader::Stored(r) => Pin::new(r).poll_read(cx, buf),
            ArchiveReader::Deflated(r) => Pin::new(r).poll_read(cx, buf),
        }
    }
}

impl<R: Unpin> Unpin for ArchiveReader<R> {}
unsafe impl<R: Send> Send for ArchiveReader<R> {}

pub struct Archive {
    reader: Reader,
    entries: Vec<FileEntry>,
}

impl Archive {
    pub async fn open(uri: &str) -> Result<Self, IoError> {
        let reader = Reader::open(uri).await?;
        let file_size = reader.file_size();

        Self::from_reader(reader, file_size).await
    }

    async fn from_reader(reader: Reader, file_size: u64) -> Result<Self, IoError> {
        let max_eocd_size = 65557;
        let eocd_start = file_size.saturating_sub(max_eocd_size);
        let eocd_size = (file_size - eocd_start) as usize;

        let eocd_buffer = reader.read_range(eocd_start, eocd_size).await?;

        let eocd_rel_offset = parser::find_eocd_offset(&eocd_buffer)
            .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to find EOCD signature."))?;

        let eocd = parser::parse_eocd(&eocd_buffer[eocd_rel_offset..])
            .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse EOCD"))?;

        let (cd_start, cd_size) = if eocd.cd_offset == 0xFFFFFFFF {
            let zip64_locator_offset = parser::find_zip64_locator(&eocd_buffer).map_err(|_| {
                IoError::new(ErrorKind::InvalidData, "Failed to find ZIP64 locator")
            })?;

            let zip64_locator = parser::parse_zip64_locator(&eocd_buffer[zip64_locator_offset..])
                .map_err(|_| {
                IoError::new(ErrorKind::InvalidData, "Failed to parse ZIP64 locator")
            })?;

            let zip64_eocd_size: usize = 56;
            let zip64_eocd_buffer = reader
                .read_range(zip64_locator.zip64_eocd_offset, zip64_eocd_size)
                .await?;

            let zip64_eocd = parser::parse_zip64_eocd(&zip64_eocd_buffer)
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse ZIP64 EOCD"))?;

            (zip64_eocd.cd_offset, zip64_eocd.cd_size as usize)
        } else {
            (eocd.cd_offset as u64, eocd.cd_size as usize)
        };

        let cd_buffer = reader.read_range(cd_start, cd_size).await?;

        let headers = parser::iterate_central_directory(&cd_buffer, eocd.total_cd_records)
            .map_err(|_| {
                IoError::new(ErrorKind::InvalidData, "Failed to parse central directory")
            })?;

        let mut entries = Vec::with_capacity(headers.len());
        for h in headers {
            entries.push(FileEntry {
                name: Arc::from(h.file_name),
                size: h.uncompressed_size as u64,
                compressed_size: h.compressed_size as u64,
                compression_method: h.compression_method,
                offset: h.local_header_offset as u64,
            });
        }

        Ok(Self { reader, entries })
    }

    pub fn list_files(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Extracts a file by name and returns a streaming `AsyncRead` handle.
    pub async fn extract_file(
        &mut self,
        filename: &str,
    ) -> Result<impl AsyncRead + Unpin + Send + 'static, IoError> {
        let target = self
            .entries
            .iter()
            .find(|e| e.name.as_ref() == filename)
            .cloned()
            .ok_or_else(|| IoError::new(ErrorKind::NotFound, "File not found in archive"))?;

        let lfh_fixed = self.reader.read_range(target.offset, 30).await?;

        if &lfh_fixed[0..4] != &[0x50, 0x4b, 0x03, 0x04] {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                "Invalid Local File Header signature",
            ));
        }

        let fn_len = u16::from_le_bytes([lfh_fixed[26], lfh_fixed[27]]) as usize;
        let extra_len = u16::from_le_bytes([lfh_fixed[28], lfh_fixed[29]]) as usize;

        let payload_offset = target.offset + 30 + fn_len as u64 + extra_len as u64;
        let range = payload_offset..payload_offset + target.compressed_size;

        let stream = self.reader.stream(range).await?;

        match target.compression_method {
            0 => {
                let reader = StreamReader::new(stream);
                Ok(ArchiveReader::Stored(reader))
            }
            8 => {
                let reader = StreamReader::new(stream);
                let decoder = DeflateDecoder::new(BufReader::new(reader));
                Ok(ArchiveReader::Deflated(decoder))
            }
            other => Err(IoError::new(
                ErrorKind::Unsupported,
                format!("Unsupported compression: {}", other),
            )),
        }
    }
}
