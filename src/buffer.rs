use crate::seeker::Seeker;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::SeekFrom;

/// RetentionBuffer acts as a smart caching layer over a Seeker.
/// It pre-fetches blocks of bytes (e.g., the last 64KB for EOCD) and serves
/// localized reads directly from memory, intercepting read/seek operations.
pub struct RetentionBuffer<S: Seeker> {
    inner: S,
    buffer: Vec<u8>,
    buffer_start_offset: u64,
    current_offset: u64,
    file_size: u64,
}

impl<S: Seeker> RetentionBuffer<S> {
    pub async fn new(inner: S, file_size: u64) -> Result<Self, IoError> {
        Ok(Self {
            inner,
            buffer: Vec::new(),
            buffer_start_offset: 0,
            current_offset: 0,
            file_size,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Fetches a specific chunk from the underlying seeker into the retention buffer.
    pub async fn fetch_chunk(&mut self, start: u64, length: usize) -> Result<(), IoError> {
        if start >= self.file_size {
            return Err(IoError::new(
                ErrorKind::InvalidInput,
                "Fetch starts past EOF",
            ));
        }

        let actual_length = std::cmp::min(length as u64, self.file_size - start) as usize;
        self.buffer.resize(actual_length, 0);

        self.inner.seek(SeekFrom::Start(start)).await?;
        let mut total_read = 0;

        while total_read < actual_length {
            let n = self.inner.read(&mut self.buffer[total_read..]).await?;
            if n == 0 {
                break; // EOF reached prematurely
            }
            total_read += n;
        }

        self.buffer.truncate(total_read);
        self.buffer_start_offset = start;
        self.current_offset = start;
        Ok(())
    }

    /// Provides direct zero-copy access to the retained memory if the span exists locally.
    /// This strictly avoids data copying.
    pub fn get_retained_slice(&self, start: u64, length: usize) -> Option<&[u8]> {
        if start >= self.buffer_start_offset
            && start + (length as u64) <= self.buffer_start_offset + (self.buffer.len() as u64)
        {
            let rel_start = (start - self.buffer_start_offset) as usize;
            Some(&self.buffer[rel_start..rel_start + length])
        } else {
            None
        }
    }

    /// Borrow the entire valid buffer slice.
    pub fn get_entire_buffer(&self) -> &[u8] {
        &self.buffer
    }

    pub fn buffer_start_offset(&self) -> u64 {
        self.buffer_start_offset
    }

    async fn internal_read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        // If the read falls entirely within our retained buffer, serve it.
        let requested_len = buf.len();
        if self.current_offset >= self.buffer_start_offset
            && self.current_offset < self.buffer_start_offset + (self.buffer.len() as u64)
        {
            let rel_start = (self.current_offset - self.buffer_start_offset) as usize;
            let available = self.buffer.len() - rel_start;
            let to_copy = std::cmp::min(requested_len, available);

            buf[..to_copy].copy_from_slice(&self.buffer[rel_start..rel_start + to_copy]);
            self.current_offset += to_copy as u64;
            return Ok(to_copy);
        }

        // If not in buffer, bypass retention (for on-the-fly uncompressed payload reading)
        self.inner
            .seek(SeekFrom::Start(self.current_offset))
            .await?;
        let n = self.inner.read(buf).await?;
        self.current_offset += n as u64;
        Ok(n)
    }

    async fn internal_seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        let new_offset = match pos {
            SeekFrom::Start(o) => o,
            SeekFrom::Current(o) => {
                if o < 0 {
                    self.current_offset.saturating_sub((-o) as u64)
                } else {
                    self.current_offset + (o as u64)
                }
            }
            SeekFrom::End(o) => {
                if o < 0 {
                    self.file_size.saturating_sub((-o) as u64)
                } else {
                    self.file_size + (o as u64)
                }
            }
        };

        self.current_offset = std::cmp::min(new_offset, self.file_size);
        Ok(self.current_offset)
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "wasi")))]
#[async_trait::async_trait]
impl<S: Seeker + Send + Sync> Seeker for RetentionBuffer<S> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.internal_read(buf).await
    }

    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        self.internal_seek(pos).await
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "wasi"))]
#[async_trait::async_trait(?Send)]
impl<S: Seeker> Seeker for RetentionBuffer<S> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.internal_read(buf).await
    }

    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        self.internal_seek(pos).await
    }
}
