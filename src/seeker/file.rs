use std::io::SeekFrom;
use std::io::{Error as IoError};
use crate::seeker::Seeker;

#[cfg(not(target_os = "wasi"))]
use tokio::fs::File;
#[cfg(not(target_os = "wasi"))]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[cfg(target_os = "wasi")]
use std::fs::File;
#[cfg(target_os = "wasi")]
use std::io::{Read, Seek};

pub struct FileSeeker {
    file: File,
}

impl FileSeeker {
    pub async fn new(path: &str) -> std::io::Result<Self> {
        #[cfg(not(target_os = "wasi"))]
        let file = File::open(path).await?;
        #[cfg(target_os = "wasi")]
        let file = File::open(path)?;
        
        Ok(Self { file })
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Seeker for FileSeeker {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        #[cfg(not(target_os = "wasi"))]
        {
            self.file.read(buf).await
        }
        #[cfg(target_os = "wasi")]
        {
            Read::read(&mut self.file, buf)
        }
    }

    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        #[cfg(not(target_os = "wasi"))]
        {
            self.file.seek(pos).await
        }
        #[cfg(target_os = "wasi")]
        {
            Seek::seek(&mut self.file, pos)
        }
    }
}
