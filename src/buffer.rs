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

#[cfg(not(target_arch = "wasm32"))]
impl<S: Seeker + Send + Sync> Seeker for RetentionBuffer<S> {
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, IoError>> + Send + 'a {
        async move { self.internal_read(buf).await }
    }

    fn seek(
        &mut self,
        pos: SeekFrom,
    ) -> impl std::future::Future<Output = Result<u64, IoError>> + Send + '_ {
        async move { self.internal_seek(pos).await }
    }
}

#[cfg(target_arch = "wasm32")]
impl<S: Seeker + Send> Seeker for RetentionBuffer<S> {
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, IoError>> + 'a {
        async move { self.internal_read(buf).await }
    }

    fn seek(
        &mut self,
        pos: SeekFrom,
    ) -> impl std::future::Future<Output = Result<u64, IoError>> + '_ {
        async move { self.internal_seek(pos).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seeker::Seeker;
    use std::io::Cursor;

    struct MockSeeker(Cursor<Vec<u8>>);

    impl Seeker for MockSeeker {
        #[cfg(not(target_arch = "wasm32"))]
        fn read<'a>(
            &'a mut self,
            buf: &'a mut [u8],
        ) -> impl std::future::Future<Output = Result<usize, IoError>> + Send + 'a {
            async move { std::io::Read::read(&mut self.0, buf) }
        }

        #[cfg(target_arch = "wasm32")]
        fn read<'a>(
            &'a mut self,
            buf: &'a mut [u8],
        ) -> impl std::future::Future<Output = Result<usize, IoError>> + 'a {
            async move { std::io::Read::read(&mut self.0, buf) }
        }

        #[cfg(not(target_arch = "wasm32"))]
        fn seek(
            &mut self,
            pos: SeekFrom,
        ) -> impl std::future::Future<Output = Result<u64, IoError>> + Send + '_ {
            async move { std::io::Seek::seek(&mut self.0, pos) }
        }

        #[cfg(target_arch = "wasm32")]
        fn seek(
            &mut self,
            pos: SeekFrom,
        ) -> impl std::future::Future<Output = Result<u64, IoError>> + '_ {
            async move { std::io::Seek::seek(&mut self.0, pos) }
        }
    }

    #[tokio::test]
    async fn test_retention_buffer_fetch_chunk() {
        let data: Vec<u8> = (0..100).collect();
        let seeker = MockSeeker(Cursor::new(data.clone()));
        let mut buffer = RetentionBuffer::new(seeker, 100).await.unwrap();

        buffer.fetch_chunk(50, 20).await.unwrap();
        assert_eq!(buffer.buffer_start_offset(), 50);

        let slice = buffer.get_retained_slice(55, 10).unwrap();
        assert_eq!(slice, &data[55..65]);

        assert!(buffer.get_retained_slice(40, 10).is_none());
    }

    #[tokio::test]
    async fn test_retention_buffer_internal_read_seek() {
        let data: Vec<u8> = (0..200).collect();
        let seeker = MockSeeker(Cursor::new(data.clone()));
        let mut buffer = RetentionBuffer::new(seeker, 200).await.unwrap();

        // Fetch piece
        buffer.fetch_chunk(100, 50).await.unwrap();

        // Read inside retained buffer
        buffer.seek(SeekFrom::Start(110)).await.unwrap();
        let mut buf = vec![0; 10];
        let n = buffer.read(&mut buf).await.unwrap();
        assert_eq!(n, 10);
        assert_eq!(buf, &data[110..120]);

        // Attempt to read OUTSIDE the retained buffer (this bypasses retention memory natively)
        buffer.seek(SeekFrom::Start(180)).await.unwrap();
        let mut buf2 = vec![0; 10];
        let n2 = buffer.read(&mut buf2).await.unwrap();
        assert_eq!(n2, 10);
        assert_eq!(buf2, &data[180..190]);
    }
}
