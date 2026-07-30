use std::io::{Error as IoError, ErrorKind};

use crate::parser;

use super::reader::Reader;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: std::sync::Arc<str>,
    pub size: u64,
    pub compressed_size: u64,
    pub compression_method: u16,
    pub offset: u64,
}

pub enum ArchiveReader<R: std::io::Read> {
    Stored(R),
    Deflated(flate2::bufread::DeflateDecoder<std::io::BufReader<R>>),
}

impl<R: std::io::Read> std::io::Read for ArchiveReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Stored(r) => r.read(buf),
            Self::Deflated(d) => d.read(buf),
        }
    }
}

unsafe impl<R: std::io::Read + Send> Send for ArchiveReader<R> {}

pub struct Archive {
    reader: Reader,
    entries: Vec<FileEntry>,
}

impl Archive {
    pub fn open(uri: &str) -> Result<Self, IoError> {
        let reader = Reader::open(uri)?;
        let file_size = reader.file_size();

        Self::from_reader(reader, file_size)
    }

    fn from_reader(reader: Reader, file_size: u64) -> Result<Self, IoError> {
        let max_eocd_size = 65557;
        let eocd_start = file_size.saturating_sub(max_eocd_size);
        let eocd_size = (file_size - eocd_start) as usize;

        let eocd_buffer = reader.read(eocd_start..eocd_start + eocd_size as u64)?;

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
            let zip64_eocd_buffer = reader.read(
                zip64_locator.zip64_eocd_offset
                    ..zip64_locator.zip64_eocd_offset + zip64_eocd_size as u64,
            )?;

            let zip64_eocd = parser::parse_zip64_eocd(&zip64_eocd_buffer)
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse ZIP64 EOCD"))?;

            (zip64_eocd.cd_offset, zip64_eocd.cd_size as usize)
        } else {
            (eocd.cd_offset as u64, eocd.cd_size as usize)
        };

        let cd_buffer = reader.read(cd_start..cd_start + cd_size as u64)?;

        let headers = parser::iterate_central_directory(&cd_buffer, eocd.total_cd_records)
            .map_err(|_| {
                IoError::new(ErrorKind::InvalidData, "Failed to parse central directory")
            })?;

        let mut entries = Vec::with_capacity(headers.len());
        for h in headers {
            // `extent`, not the raw 32-bit fields: a zip64 entry keeps its real
            // sizes in the extra field and the header carries only a sentinel.
            let extent = h.extent();
            entries.push(FileEntry {
                name: std::sync::Arc::from(h.file_name),
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

    pub fn extract_file(
        &mut self,
        filename: &str,
    ) -> Result<impl std::io::Read + Send + 'static, IoError> {
        let target = self
            .entries
            .iter()
            .find(|e| e.name.as_ref() == filename)
            .cloned()
            .ok_or_else(|| IoError::new(ErrorKind::NotFound, "File not found in archive"))?;

        let lfh_fixed = self.reader.read(target.offset..target.offset + 30)?;

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

        let stream = self.reader.into_read(range)?;

        match target.compression_method {
            0 => Ok(ArchiveReader::Stored(stream)),
            8 => {
                let buffered = std::io::BufReader::with_capacity(128 * 1024, stream);

                Ok(ArchiveReader::Deflated(
                    flate2::bufread::DeflateDecoder::new(buffered),
                ))
            }
            other => Err(IoError::new(
                ErrorKind::Unsupported,
                format!("Unsupported compression: {}", other),
            )),
        }
    }
}
