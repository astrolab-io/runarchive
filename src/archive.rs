#[cfg(feature = "async-futures")]
use async_compression::futures::bufread::DeflateDecoder;
#[cfg(feature = "async-tokio")]
use async_compression::tokio::bufread::DeflateDecoder;

#[cfg(feature = "async-futures")]
use futures::AsyncRead;
#[cfg(feature = "async-tokio")]
use tokio::io::{AsyncRead, ReadBuf};

use pin_project_lite::pin_project;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[cfg(feature = "async-tokio")]
type StreamReaderType = tokio_util::io::StreamReader<super::reader::ByteStream, bytes::Bytes>;

#[cfg(feature = "async-tokio")]
pin_project! {
    pub struct ArchiveReader {
        #[pin]
        pub inner: DeflateDecoder<StreamReaderType>,
    }
}

#[cfg(feature = "async-futures")]
pin_project! {
    pub struct ArchiveReader<R> {
        #[pin]
        pub inner: DeflateDecoder<R>,
    }
}

#[cfg(feature = "async-tokio")]
impl AsyncRead for ArchiveReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.project().inner.poll_read(cx, buf)
    }
}

#[cfg(feature = "async-futures")]
impl<R: futures::AsyncBufRead> AsyncRead for ArchiveReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        self.project().inner.poll_read(cx, buf)
    }
}

#[cfg(feature = "async-tokio")]
unsafe impl Send for ArchiveReader {}

#[cfg(feature = "async-futures")]
unsafe impl<R: futures::AsyncBufRead> Send for ArchiveReader<R> {}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: Arc<str>,
    pub size: u64,
    pub compressed_size: u64,
    pub compression_method: u16,
    pub offset: u64,
}

use super::reader::Reader;
use std::io::{Error as IoError, ErrorKind};

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

        let eocd_buffer = reader
            .read(eocd_start..eocd_start + eocd_size as u64)
            .await?;

        let eocd_rel_offset = crate::parser::find_eocd_offset(&eocd_buffer)
            .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to find EOCD signature."))?;

        let eocd = crate::parser::parse_eocd(&eocd_buffer[eocd_rel_offset..])
            .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse EOCD"))?;

        let (cd_start, cd_size) = if eocd.cd_offset == 0xFFFFFFFF {
            let zip64_locator_offset =
                crate::parser::find_zip64_locator(&eocd_buffer).map_err(|_| {
                    IoError::new(ErrorKind::InvalidData, "Failed to find ZIP64 locator")
                })?;

            let zip64_locator = crate::parser::parse_zip64_locator(
                &eocd_buffer[zip64_locator_offset..],
            )
            .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse ZIP64 locator"))?;

            let zip64_eocd_size: usize = 56;
            let zip64_eocd_buffer = reader
                .read(
                    zip64_locator.zip64_eocd_offset
                        ..zip64_locator.zip64_eocd_offset + zip64_eocd_size as u64,
                )
                .await?;

            let zip64_eocd = crate::parser::parse_zip64_eocd(&zip64_eocd_buffer)
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse ZIP64 EOCD"))?;

            (zip64_eocd.cd_offset, zip64_eocd.cd_size as usize)
        } else {
            (eocd.cd_offset as u64, eocd.cd_size as usize)
        };

        let cd_buffer = reader.read(cd_start..cd_start + cd_size as u64).await?;

        let headers = crate::parser::iterate_central_directory(&cd_buffer, eocd.total_cd_records)
            .map_err(|_| {
            IoError::new(ErrorKind::InvalidData, "Failed to parse central directory")
        })?;

        let mut entries = Vec::with_capacity(headers.len());
        for h in headers {
            // `extent`, not the raw 32-bit fields: a zip64 entry keeps its real
            // sizes in the extra field and the header carries only a sentinel.
            let extent = h.extent();
            entries.push(FileEntry {
                name: Arc::from(h.file_name),
                size: extent.uncompressed_size,
                compressed_size: extent.compressed_size,
                compression_method: h.compression_method,
                offset: extent.local_header_offset,
            });
        }

        Ok(Self { reader, entries })
    }

    pub fn list_files(&self) -> &[FileEntry] {
        &self.entries
    }

    #[cfg(feature = "async-tokio")]
    pub async fn extract_file(
        &mut self,
        filename: &str,
    ) -> Result<impl tokio::io::AsyncRead + Unpin + Send + 'static, IoError> {
        let target = self
            .entries
            .iter()
            .find(|e| e.name.as_ref() == filename)
            .cloned()
            .ok_or_else(|| IoError::new(ErrorKind::NotFound, "File not found in archive"))?;

        let lfh_fixed = self.reader.read(target.offset..target.offset + 30).await?;

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

        let stream = self.reader.read_to_stream(range).await?;
        use tokio_util::io::StreamReader;
        let stream_reader: StreamReader<_, bytes::Bytes> = StreamReader::new(stream);

        let result: Box<dyn tokio::io::AsyncRead + Unpin + Send + 'static> =
            match target.compression_method {
                0 => Box::new(stream_reader),
                8 => Box::new(ArchiveReader {
                    inner: DeflateDecoder::new(stream_reader) as DeflateDecoder<StreamReaderType>,
                }),
                other => {
                    return Err(IoError::new(
                        ErrorKind::Unsupported,
                        format!("Unsupported compression: {}", other),
                    ))
                }
            };
        Ok(result)
    }

    #[cfg(feature = "async-futures")]
    pub async fn extract_file(
        &mut self,
        filename: &str,
    ) -> Result<impl futures::AsyncRead + Unpin + Send + 'static, IoError> {
        let target = self
            .entries
            .iter()
            .find(|e| e.name.as_ref() == filename)
            .cloned()
            .ok_or_else(|| IoError::new(ErrorKind::NotFound, "File not found in archive"))?;

        let lfh_fixed = self.reader.read(target.offset..target.offset + 30).await?;

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

        #[cfg(feature = "async-futures")]
        let async_bufread = self.reader.into_async_bufread(range).await?;

        let result: Box<dyn futures::AsyncRead + Unpin + Send + 'static> =
            match target.compression_method {
                0 => Box::new(async_bufread),
                8 => Box::new(ArchiveReader {
                    inner: DeflateDecoder::new(async_bufread),
                }),
                other => {
                    return Err(IoError::new(
                        ErrorKind::Unsupported,
                        format!("Unsupported compression: {}", other),
                    ))
                }
            };
        Ok(result)
    }
}
