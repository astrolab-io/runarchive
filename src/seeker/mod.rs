use std::io::{Error as IoError, SeekFrom};
use async_trait::async_trait;

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Seeker {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError>;
}

#[cfg(not(target_arch = "wasm32"))]
pub type SeekerBox = Box<dyn Seeker + Send + Sync + Unpin>;

#[cfg(target_arch = "wasm32")]
pub type SeekerBox = Box<dyn Seeker + Unpin>;

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl<S: ?Sized + Seeker + Unpin + Send> Seeker for Box<S> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        (**self).read(buf).await
    }
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        (**self).seek(pos).await
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl<S: ?Sized + Seeker + Unpin> Seeker for Box<S> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        (**self).read(buf).await
    }
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        (**self).seek(pos).await
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod file;

pub mod http;
