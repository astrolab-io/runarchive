pub mod file;
pub mod http;

use async_trait::async_trait;
use std::io::SeekFrom;
use std::io::Error as IoError;

#[async_trait]
pub trait Seeker {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError>;
}

#[async_trait]
impl<S: ?Sized + Seeker + Unpin + Send> Seeker for Box<S> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        (**self).read(buf).await
    }
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
        (**self).seek(pos).await
    }
}
