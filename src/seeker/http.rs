use crate::seeker::Seeker;
use std::io::SeekFrom;
use std::io::{Error as IoError, ErrorKind};

#[cfg(not(target_os = "wasi"))]
use reqwest::Client;

#[cfg(target_os = "wasi")]
use wstd::http::{Client, Request};

pub struct HttpSeeker {
    client: Client,
    url: String,
    auth: Option<String>,
    current_offset: u64,
    file_size: u64,
}

impl HttpSeeker {
    pub async fn new(url_str: &str) -> Result<Self, IoError> {
        let (url_clean, auth) = extract_auth(url_str);

        let url = url_clean;

        #[cfg(not(target_os = "wasi"))]
        {
            // Validation for reqwest
            let _ = url::Url::parse(&url).map_err(|e| IoError::new(ErrorKind::InvalidInput, e))?;
        }

        #[cfg(not(target_arch = "wasm32"))]
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .build()
            .map_err(|e| IoError::new(ErrorKind::Other, e))?;

        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        let client = Client::new();

        #[cfg(target_os = "wasi")]
        let client = Client::new();

        let mut file_size = 0;

        #[cfg(not(target_os = "wasi"))]
        {
            let mut req_builder = client.head(&url);
            if let Some(ref a) = auth {
                req_builder = req_builder.header(reqwest::header::AUTHORIZATION, a);
            }
            let head_resp = req_builder
                .send()
                .await
                .map_err(|e| IoError::new(ErrorKind::ConnectionAborted, e))?;

            if head_resp.status().is_success() {
                if let Some(len) = head_resp.content_length() {
                    file_size = len;
                }
            }

            // If HEAD failed or gave 0 length, fallback to GET Range=0-0
            if file_size == 0 {
                let mut req_builder = client.get(&url).header(reqwest::header::RANGE, "bytes=0-0");
                if let Some(ref a) = auth {
                    req_builder = req_builder.header(reqwest::header::AUTHORIZATION, a);
                }
                let get_resp = req_builder
                    .send()
                    .await
                    .map_err(|e| IoError::new(ErrorKind::ConnectionAborted, e))?;

                if get_resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                    if let Some(cr) = get_resp.headers().get(reqwest::header::CONTENT_RANGE) {
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

        #[cfg(target_os = "wasi")]
        {
            let mut req_builder = Request::head(&url);
            if let Some(ref a) = auth {
                req_builder = req_builder.header("authorization", a);
            }
            let req = req_builder
                .body(wstd::http::Body::empty())
                .map_err(|e| IoError::new(ErrorKind::Other, e))?;
            let resp = client
                .send(req)
                .await
                .map_err(|e| IoError::new(ErrorKind::ConnectionAborted, e))?;

            if resp.status().is_success() {
                if let Some(len_val) = resp.headers().get("content-length") {
                    if let Ok(len_str) = len_val.to_str() {
                        if let Ok(len) = len_str.parse::<u64>() {
                            file_size = len;
                        }
                    }
                }
            } else if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
                return Err(IoError::new(
                    ErrorKind::PermissionDenied,
                    format!("Authentication failed with status: {} for {}", resp.status(), url),
                ));
            }

            if file_size == 0 {
                let mut req_builder = Request::get(&url).header("range", "bytes=0-0");
                if let Some(ref a) = auth {
                    req_builder = req_builder.header("authorization", a);
                }
                let req = req_builder
                    .body(wstd::http::Body::empty())
                    .map_err(|e| IoError::new(ErrorKind::Other, e))?;
                let resp = client
                    .send(req)
                    .await
                    .map_err(|e| IoError::new(ErrorKind::ConnectionAborted, e))?;

                if resp.status().as_u16() == 206 {
                    // PARTIAL_CONTENT
                    if let Some(cr) = resp.headers().get("content-range") {
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
                "Failed to determine remote file size",
            ));
        }

        Ok(Self {
            client,
            url,
            auth,
            current_offset: 0,
            file_size,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub async fn new_with_size(url_str: &str, file_size: u64) -> Result<Self, IoError> {
        let (url_clean, auth) = extract_auth(url_str);

        let url = url_clean;

        #[cfg(not(target_os = "wasi"))]
        {
            let _ = url::Url::parse(&url).map_err(|e| IoError::new(ErrorKind::InvalidInput, e))?;
        }

        Ok(Self {
            client: Client::new(),
            url,
            auth,
            current_offset: 0,
            file_size,
        })
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Seeker for HttpSeeker {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.current_offset >= self.file_size {
            return Ok(0);
        }

        let start = self.current_offset;
        let mut end = start + buf.len() as u64 - 1;
        if end >= self.file_size {
            end = self.file_size - 1;
        }

        let range_header = format!("bytes={}-{}", start, end);

        #[cfg(not(target_os = "wasi"))]
        {
            let mut req_builder = self.client.get(&self.url).header(reqwest::header::RANGE, range_header);
            if let Some(ref a) = self.auth {
                req_builder = req_builder.header(reqwest::header::AUTHORIZATION, a);
            }
            let resp = req_builder
                .send()
                .await
                .map_err(|e| IoError::new(ErrorKind::ConnectionAborted, e))?;

            if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err(IoError::new(
                    ErrorKind::Unsupported,
                    "Server did not return 206 Partial Content",
                ));
            }

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| IoError::new(ErrorKind::InvalidData, e))?;
            let len = bytes.len();

            if len > buf.len() {
                return Err(IoError::new(
                    ErrorKind::InvalidData,
                    "Received more bytes than requested",
                ));
            }

            buf[..len].copy_from_slice(&bytes);
            self.current_offset += len as u64;

            Ok(len)
        }

        #[cfg(target_os = "wasi")]
        {
            let mut req_builder = Request::get(&self.url).header("range", range_header);
            if let Some(ref a) = self.auth {
                req_builder = req_builder.header("authorization", a);
            }
            let req = req_builder
                .body(wstd::http::Body::empty())
                .map_err(|e| IoError::new(ErrorKind::Other, e))?;
            let resp = self
                .client
                .send(req)
                .await
                .map_err(|e| IoError::new(ErrorKind::ConnectionAborted, e))?;

            if resp.status().as_u16() != 206 {
                return Err(IoError::new(
                    ErrorKind::Unsupported,
                    "Server did not return 206 Partial Content",
                ));
            }

            let mut body = resp.into_body();
            let bytes = body
                .contents()
                .await
                .map_err(|e| IoError::new(ErrorKind::InvalidData, e))?;
            let len = bytes.len();

            if len > buf.len() {
                return Err(IoError::new(
                    ErrorKind::InvalidData,
                    "Received more bytes than requested",
                ));
            }

            buf[..len].copy_from_slice(&bytes);
            self.current_offset += len as u64;

            Ok(len)
        }
    }

    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
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

fn extract_auth(url_str: &str) -> (String, Option<String>) {
    use url::Url;
    if let Ok(mut url) = Url::parse(url_str) {
        if !url.username().is_empty() || url.password().is_some() {
            let user = url.username();
            let pass = url.password().unwrap_or("");

            use base64::prelude::*;
            let auth = format!(
                "Basic {}",
                BASE64_STANDARD.encode(format!("{}:{}", user, pass))
            );

            // Reconstruct URL without credentials
            let _ = url.set_username("");
            let _ = url.set_password(None);

            return (url.to_string(), Some(auth));
        }
    }
    (url_str.to_string(), None)
}
