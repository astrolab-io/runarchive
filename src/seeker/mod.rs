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
    #[cfg(not(target_os = "wasi"))]
    Http(http::HttpSeeker),
    #[cfg(target_os = "wasi")]
    WasiHttp(wasi_http::WasiHttpSeeker),
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
                #[cfg(not(target_os = "wasi"))]
                SeekerImpl::Http(s) => s.read(buf).await,
                #[cfg(target_os = "wasi")]
                SeekerImpl::WasiHttp(s) => s.read(buf).await,
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
                #[cfg(not(target_os = "wasi"))]
                SeekerImpl::Http(s) => s.read(buf).await,
                #[cfg(target_os = "wasi")]
                SeekerImpl::WasiHttp(s) => s.read(buf).await,
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
                #[cfg(not(target_os = "wasi"))]
                SeekerImpl::Http(s) => s.seek(pos).await,
                #[cfg(target_os = "wasi")]
                SeekerImpl::WasiHttp(s) => s.seek(pos).await,
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
                #[cfg(not(target_os = "wasi"))]
                SeekerImpl::Http(s) => s.seek(pos).await,
                #[cfg(target_os = "wasi")]
                SeekerImpl::WasiHttp(s) => s.seek(pos).await,
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod file;

#[cfg(not(target_os = "wasi"))]
pub mod http;

#[cfg(target_os = "wasi")]
pub mod wasi_http;
