use crate::buffer::RetentionBuffer;
use crate::seeker::{Seeker, SeekerBox};
use crate::parser;
use std::io::{Error as IoError, ErrorKind};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub compression_method: u16,
    pub offset: u64,
}

pub struct Archive {
    buffer: RetentionBuffer<SeekerBox>,
    entries: Vec<FileEntry>,
}

impl Archive {
    pub async fn open(uri: &str) -> Result<Self, IoError> {
        #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
        {
            let is_http = uri.starts_with("http://") || uri.starts_with("https://");

            let (seeker, file_size) = if is_http {
                let s = crate::seeker::http::HttpSeeker::new(uri).await?;
                let size = s.file_size();
                (Box::new(s) as SeekerBox, size)
            } else {
                let file_path = if let Some(stripped) = uri.strip_prefix("file://") {
                    stripped.to_string()
                } else {
                    uri.to_string()
                };

                let s = crate::seeker::file::FileSeeker::new(&file_path).await?;
                
                #[cfg(not(target_os = "wasi"))]
                let size = tokio::fs::metadata(&file_path).await?.len();
                #[cfg(target_os = "wasi")]
                let size = {
                    let m = std::fs::metadata(&file_path)?;
                    m.len()
                };
                
                (Box::new(s) as SeekerBox, size)
            };

            Self::from_seeker(seeker, file_size).await
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let s = crate::seeker::http::HttpSeeker::new(uri).await?;
            let size = s.file_size();
            let seeker = Box::new(s) as SeekerBox;
            Self::from_seeker(seeker, size).await
        }
    }

    /// Internal initializer from an already resolved seeker
    async fn from_seeker(seeker: SeekerBox, file_size: u64) -> Result<Self, IoError> {
        let mut buffer = RetentionBuffer::new(seeker, file_size).await?;
        
        let max_eocd_size = 65557;
        let fetch_start = file_size.saturating_sub(max_eocd_size);
        let fetch_len = (file_size - fetch_start) as usize;
        
        buffer.fetch_chunk(fetch_start, fetch_len).await?;
        
        // Block to limit the immutable borrow of `buffer`
        let parsed_headers = {
            let slice = buffer.get_retained_slice(fetch_start, fetch_len)
                .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "Failed to get EOCD slice"))?;
            let eocd_rel_offset = parser::find_eocd_offset(slice)
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to find EOCD signature."))?;
            let eocd = parser::parse_eocd(&slice[eocd_rel_offset..])
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse EOCD"))?;
            
            buffer.fetch_chunk(eocd.cd_offset as u64, eocd.cd_size as usize).await?;
            let cd_slice = buffer.get_retained_slice(eocd.cd_offset as u64, eocd.cd_size as usize)
                .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "Failed to get CD slice"))?;
            
            // We parse and immediately convert to owned structures to release the borrow on the buffer
            // so we can use the buffer for data extraction later. Filenames are small, so allocating
            // them is cheap and avoids complex self-referential lifetimes.
            let headers = parser::iterate_central_directory(cd_slice, eocd.total_cd_records)
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse central directory"))?;
                
            let mut entries = Vec::with_capacity(headers.len());
            for h in headers {
                entries.push(FileEntry {
                    name: h.file_name.to_string(),
                    size: h.uncompressed_size as u64,
                    compressed_size: h.compressed_size as u64,
                    compression_method: h.compression_method,
                    offset: h.local_header_offset as u64,
                });
            }
            entries
        };
        
        Ok(Self {
            buffer,
            entries: parsed_headers,
        })
    }

    /// Returns a list of cached file entries in the archive
    pub fn list_files(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Extracts a specific file by name to the provided AsyncWrite stream
    #[cfg(not(target_os = "wasi"))]
    pub async fn extract_file<W>(&mut self, filename: &str, mut writer: W) -> Result<(), IoError> 
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        let target = self.entries.iter().find(|e| e.name == filename).cloned();
        let target = target.ok_or_else(|| IoError::new(ErrorKind::NotFound, "File not found in archive"))?;
        
        self.buffer.seek(std::io::SeekFrom::Start(target.offset)).await?;
        
        let mut lfh_fixed = vec![0u8; 30];
        self.buffer.read(&mut lfh_fixed).await?;
        
        if &lfh_fixed[0..4] != &[0x50, 0x4b, 0x03, 0x04] {
            return Err(IoError::new(ErrorKind::InvalidData, "Invalid Local File Header signature"));
        }
        
        let fn_len = u16::from_le_bytes([lfh_fixed[26], lfh_fixed[27]]) as usize;
        let extra_len = u16::from_le_bytes([lfh_fixed[28], lfh_fixed[29]]) as usize;
        
        let payload_offset = target.offset + 30 + fn_len as u64 + extra_len as u64;
        self.buffer.seek(std::io::SeekFrom::Start(payload_offset)).await?;
        
        if target.compression_method == 0 {
            let mut remaining = target.compressed_size;
            let mut chunk = vec![0u8; 65536];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = self.buffer.read(&mut chunk[..to_read]).await?;
                if n == 0 { break; }
                
                use tokio::io::AsyncWriteExt;
                writer.write_all(&chunk[..n]).await?;
                
                remaining -= n as u64;
            }
        } else if target.compression_method == 8 {
            use tokio::io::AsyncWriteExt;
            let mut decoder = async_compression::tokio::write::DeflateDecoder::new(writer);
            let mut remaining = target.compressed_size;
            let mut chunk = vec![0u8; 65536];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = self.buffer.read(&mut chunk[..to_read]).await?;
                if n == 0 { break; }
                decoder.write_all(&chunk[..n]).await?;
                remaining -= n as u64;
            }
            decoder.shutdown().await?;
        } else {
            return Err(IoError::new(ErrorKind::Unsupported, format!("Unsupported compression method: {}", target.compression_method)));
        }
        
        Ok(())
    }

    /// Extracts a specific file by name to the provided AsyncWrite stream
    #[cfg(target_os = "wasi")]
    pub async fn extract_file<W>(&mut self, filename: &str, mut writer: W) -> Result<(), IoError> 
    where
        W: wstd::io::AsyncWrite + Unpin,
    {
        let target = self.entries.iter().find(|e| e.name == filename).cloned();
        let target = target.ok_or_else(|| IoError::new(ErrorKind::NotFound, "File not found in archive"))?;
        
        self.buffer.seek(std::io::SeekFrom::Start(target.offset)).await?;
        
        let mut lfh_fixed = vec![0u8; 30];
        self.buffer.read(&mut lfh_fixed).await?;
        
        if &lfh_fixed[0..4] != &[0x50, 0x4b, 0x03, 0x04] {
            return Err(IoError::new(ErrorKind::InvalidData, "Invalid Local File Header signature"));
        }
        
        let fn_len = u16::from_le_bytes([lfh_fixed[26], lfh_fixed[27]]) as usize;
        let extra_len = u16::from_le_bytes([lfh_fixed[28], lfh_fixed[29]]) as usize;
        
        let payload_offset = target.offset + 30 + fn_len as u64 + extra_len as u64;
        self.buffer.seek(std::io::SeekFrom::Start(payload_offset)).await?;
        
        if target.compression_method == 0 {
            let mut remaining = target.compressed_size;
            let mut chunk = vec![0u8; 65536];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = self.buffer.read(&mut chunk[..to_read]).await?;
                if n == 0 { break; }
                
                let mut written = 0;
                while written < n {
                    let m = writer.write(&chunk[written..n]).await?;
                    if m == 0 { return Err(IoError::new(ErrorKind::WriteZero, "Failed to write whole chunk")); }
                    written += m;
                }
                
                remaining -= n as u64;
            }
        } else if target.compression_method == 8 {
            use flate2::{Decompress, FlushDecompress, Status};
            let mut decompressor = Decompress::new(false);
            let mut remaining = target.compressed_size;
            let mut chunk = vec![0u8; 65536];
            let mut out_buffer = vec![0u8; 65536];
            
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = self.buffer.read(&mut chunk[..to_read]).await?;
                if n == 0 { break; }
                
                let mut consumed = 0;
                while consumed < n {
                    let before_in = decompressor.total_in();
                    let before_out = decompressor.total_out();
                    
                    let status = decompressor.decompress(
                        &chunk[consumed..n],
                        &mut out_buffer,
                        FlushDecompress::None,
                    ).map_err(|e| IoError::new(ErrorKind::InvalidData, e))?;
                    
                    let produced = (decompressor.total_out() - before_out) as usize;
                    consumed += (decompressor.total_in() - before_in) as usize;
                    
                    if produced > 0 {
                        let mut written = 0;
                        while written < produced {
                            let m = writer.write(&out_buffer[written..produced]).await?;
                            if m == 0 { return Err(IoError::new(ErrorKind::WriteZero, "Failed to write whole chunk")); }
                            written += m;
                        }
                    }
                    
                    if status == Status::StreamEnd {
                        break;
                    }
                }
                remaining -= n as u64;
            }
        } else {
            return Err(IoError::new(ErrorKind::Unsupported, format!("Unsupported compression method: {}", target.compression_method)));
        }
        
        Ok(())
    }
}
