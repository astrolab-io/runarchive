use std::io::SeekFrom;
use std::io::Error as IoError;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use crate::seeker::Seeker;

pub struct FileSeeker {
    file: File,
}

impl FileSeeker {
    pub async fn new(path: &str) -> std::io::Result<Self> {
        let file = File::open(path).await?;
        Ok(Self { file })
    }
}

#[async_trait::async_trait]
impl Seeker for FileSeeker {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.file.read(buf).await
    }

    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        self.file.seek(pos).await
    }
}
