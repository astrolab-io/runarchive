use crate::seeker::Seeker;
use std::io::SeekFrom;
use std::io::{Error as IoError, ErrorKind};
use wstd::http::{Client, Request};

pub struct WasiHttpSeeker {
    url: String,
    current_offset: u64,
    file_size: u64,
}

impl WasiHttpSeeker {
    pub async fn new(url: String) -> Result<Self, IoError> {
        let mut file_size = 0;
        let client = Client::new();

        let req = Request::builder()
            .method("HEAD")
            .uri(&url)
            .body(wstd::http::Body::empty())
            .map_err(|e| IoError::new(ErrorKind::Other, e.to_string()))?;

        if let Ok(response) = client.send(req).await {
            if response.status().is_success() {
                if let Some(len) = response.headers().get("content-length") {
                    if let Ok(len_str) = len.to_str() {
                        if let Ok(l) = len_str.parse::<u64>() {
                            file_size = l;
                        }
                    }
                }
            }
        }

        if file_size == 0 {
            let req = Request::builder()
                .method("GET")
                .uri(&url)
                .header("range", "bytes=0-0")
                .body(wstd::http::Body::empty())
                .map_err(|e| IoError::new(ErrorKind::Other, e.to_string()))?;

            if let Ok(response) = client.send(req).await {
                if response.status() == 206 {
                    // Partial Content
                    if let Some(cr) = response.headers().get("content-range") {
                        if let Ok(cr_str) = cr.to_str() {
                            if let Some(idx) = cr_str.find('/') {
                                if let Ok(size) = cr_str[idx + 1..].parse::<u64>() {
                                    file_size = size;
                                }
                            }
                        }
                    }
                }
            }
        }

        if file_size == 0 {
            return Err(IoError::new(
                ErrorKind::Unsupported,
                "Failed to determine remote file size using wasi:http",
            ));
        }

        Ok(Self {
            url,
            current_offset: 0,
            file_size,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }
}

impl Seeker for WasiHttpSeeker {
    fn read<'a>(
        &'a mut self,
        buf: &'a mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, IoError>> + 'a {
        async move {
            if buf.is_empty() || self.current_offset >= self.file_size {
                return Ok(0);
            }

            let start = self.current_offset;
            let mut end = start + buf.len() as u64 - 1;
            if end >= self.file_size {
                end = self.file_size - 1;
            }

            let range_header = format!("bytes={}-{}", start, end);
            let req = Request::builder()
                .method("GET")
                .uri(&self.url)
                .header("range", range_header)
                .body(wstd::http::Body::empty())
                .map_err(|e| IoError::new(ErrorKind::Other, e.to_string()))?;

            let client = Client::new();
            let response = client
                .send(req)
                .await
                .map_err(|e| IoError::new(ErrorKind::Other, e.to_string()))?;

            if !response.status().is_success() {
                return Err(IoError::new(
                    ErrorKind::ConnectionAborted,
                    format!("WASI HTTP request failed: status {}", response.status()),
                ));
            }

            let mut body = response.into_body();
            let contents = body
                .contents()
                .await
                .map_err(|e| IoError::new(ErrorKind::Other, e.to_string()))?;

            let n = std::cmp::min(buf.len(), contents.len());
            buf[..n].copy_from_slice(&contents[..n]);

            self.current_offset += n as u64;
            Ok(n)
        }
    }

    fn seek(
        &mut self,
        pos: SeekFrom,
    ) -> impl std::future::Future<Output = Result<u64, IoError>> + '_ {
        async move {
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
}
