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

impl Seeker for FileSeeker {
    #[cfg(not(target_arch = "wasm32"))]
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> impl std::future::Future<Output = Result<usize, IoError>> + Send + 'a {
        async move { self.file.read(buf).await }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn seek(&mut self, pos: SeekFrom) -> impl std::future::Future<Output = Result<u64, IoError>> + Send + '_ {
        async move { self.file.seek(pos).await }
    }
}
