use std::io::{Error as IoError, SeekFrom};

pub trait Seeker {
    #[cfg(not(target_arch = "wasm32"))]
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, IoError>> + Send + 'a;
    #[cfg(target_arch = "wasm32")]
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, IoError>> + 'a;

    #[cfg(not(target_arch = "wasm32"))]
    fn seek(
        &mut self,
        pos: SeekFrom,
    ) -> impl std::future::Future<Output = Result<u64, IoError>> + Send + '_;
    #[cfg(target_arch = "wasm32")]
    fn seek(
        &mut self,
        pos: SeekFrom,
    ) -> impl std::future::Future<Output = Result<u64, IoError>> + '_;
}

pub enum SeekerImpl {
    #[cfg(not(target_arch = "wasm32"))]
    File(file::FileSeeker),
    Http(http::HttpSeeker),
}

impl Seeker for SeekerImpl {
    #[cfg(not(target_arch = "wasm32"))]
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, IoError>> + Send + 'a {
        async move {
            match self {
                SeekerImpl::File(s) => s.read(buf).await,
                SeekerImpl::Http(s) => s.read(buf).await,
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, IoError>> + 'a {
        async move {
            match self {
                SeekerImpl::Http(s) => s.read(buf).await,
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn seek(
        &mut self,
        pos: SeekFrom,
    ) -> impl std::future::Future<Output = Result<u64, IoError>> + Send + '_ {
        async move {
            match self {
                SeekerImpl::File(s) => s.seek(pos).await,
                SeekerImpl::Http(s) => s.seek(pos).await,
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn seek(
        &mut self,
        pos: SeekFrom,
    ) -> impl std::future::Future<Output = Result<u64, IoError>> + '_ {
        async move {
            match self {
                SeekerImpl::Http(s) => s.seek(pos).await,
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod file;

pub mod http;
