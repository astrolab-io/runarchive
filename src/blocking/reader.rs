use bytes::Bytes;
use opendal::blocking::Reader as OpendalBlockingReader;
use std::{
    io::{Error as IoError, ErrorKind},
    ops::RangeBounds,
};

use url::Url;

fn split_zip_uri(uri: &str) -> Option<(String, String)> {
    let url = Url::parse(uri).ok()?;

    let segments: Vec<&str> = url.path_segments()?.collect();

    let filename = segments.last()?.to_string();

    let parent_path = if segments.len() == 1 {
        "/".to_string()
    } else {
        let mut path = String::new();
        for seg in &segments[..segments.len() - 1] {
            path.push('/');
            path.push_str(seg);
        }
        path
    };

    let mut url = url;
    url.set_path(&parent_path);
    Some((url.to_string(), filename))
}

pub struct Reader {
    inner: OpendalBlockingReader,
    file_size: u64,
}

impl Reader {
    pub fn open(uri: &str) -> Result<Self, IoError> {
        let (uri, path) = split_zip_uri(uri)
            .ok_or_else(|| IoError::new(ErrorKind::InvalidInput, "Invalid ZIP URI"))?;

        // Retry/resume policy lives in `crate::operator`, shared with the async
        // reader. Wrapping it for blocking use needs an ambient tokio handle —
        // the same requirement `blocking::Operator::from_uri` had.
        let operator = opendal::blocking::Operator::new(crate::operator::build(uri.as_str())?)
            .map_err(|e| {
                IoError::new(
                    ErrorKind::Other,
                    format!("Failed to create operator: {}", e),
                )
            })?;

        let meta = operator
            .stat(&path)
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Failed to stat file: {}", e)))?;

        let file_size = meta.content_length();

        // Chunked, so no single request spans a whole member — see
        // `crate::operator`.
        let reader = operator
            .reader_options(&path, crate::operator::reader_options())
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Failed to open reader: {}", e)))?;

        Ok(Self {
            inner: reader,
            file_size,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn read(&self, range: impl RangeBounds<u64>) -> Result<Bytes, IoError> {
        self.inner
            .read(range)
            .map(|b| b.to_bytes())
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Read error: {}", e)))
    }

    pub fn into_read(
        &self,
        range: impl RangeBounds<u64>,
    ) -> Result<impl std::io::Read + Send + 'static, IoError> {
        self.inner
            .clone()
            .into_std_read(range)
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Into std read error: {}", e)))
    }
}
