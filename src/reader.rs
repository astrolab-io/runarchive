use bytes::Bytes;
use futures::stream::{Stream, TryStreamExt};
use futures::StreamExt;
use opendal::{Operator, Reader as FileReader};
use std::{
    io::{Error as IoError, ErrorKind},
    ops::RangeBounds,
    pin::Pin,
};
use url::Url;

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<bytes::Bytes, IoError>> + Send>>;

/// If the given URI ends with a `.zip` file name, returns:
/// - The URI of the parent directory (with same query/fragment).
/// - The filename (the last path segment, percent‑decoded).
/// Otherwise returns `None`.
pub fn split_zip_uri(uri: &str) -> Option<(String, String)> {
    let mut url = Url::parse(uri).ok()?;

    // Get the decoded path segments.
    let segments: Vec<&str> = url.path_segments()?.collect();

    // Check last segment and extract filename.
    let filename = segments.last()?;
    if !filename.ends_with(".zip") {
        return None;
    }

    // Remove the last segment to get the directory path.
    let mut parent_segments = segments;
    let filename_owned = parent_segments.pop().unwrap().to_string(); // safe because we checked last()

    // Rebuild the path. Empty path becomes "/" (root directory).
    let parent_path = if parent_segments.is_empty() {
        "/".to_string()
    } else {
        let mut path = String::new();
        for seg in parent_segments {
            path.push('/');
            path.push_str(seg);
        }
        path
    };

    url.set_path(&parent_path);
    Some((url.to_string(), filename_owned))
}

/// Reader wraps an OpenDAL Reader and provides range-based access
/// with a known file_size for archive parsing operations.
pub struct Reader {
    inner: FileReader,
    file_size: u64,
}

impl Reader {
    /// Opens an archive from a URI. If the URI points to a .zip file,
    /// it splits the URI to get the parent directory operator and the filename path.
    /// OpenDAL handles the protocol detection automatically.
    pub async fn open(uri: &str) -> Result<Self, IoError> {
        if let Some((uri, path)) = split_zip_uri(uri) {
            Self::from_parts(&uri, &path).await
        } else {
            Err(IoError::new(ErrorKind::InvalidInput, "Invalid ZIP URI"))
        }
    }

    async fn from_parts(uri: &str, path: &str) -> Result<Self, IoError> {
        let operator = Operator::from_uri(uri).map_err(|e| {
            IoError::new(
                ErrorKind::Other,
                format!("Failed to create operator: {}", e),
            )
        })?;

        // Stat the file to get its size
        let meta = operator
            .stat(&path)
            .await
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Failed to stat file: {}", e)))?;

        let file_size = meta.content_length();

        // Get the async reader from OpenDAL
        let reader = operator
            .reader(&path)
            .await
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Failed to open reader: {}", e)))?;

        Ok(Self {
            inner: reader,
            file_size,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Read a range of bytes from the file efficiently
    pub async fn read_range(&self, offset: u64, size: usize) -> Result<Bytes, IoError> {
        let buf = self
            .inner
            .read(offset..offset + size as u64)
            .await
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Failed to read range: {}", e)))?;
        Ok(buf.to_bytes())
    }

    pub async fn stream(&self, range: impl RangeBounds<u64>) -> Result<ByteStream, IoError> {
        let inner = self.inner.clone();
        let stream = inner
            .into_bytes_stream(range)
            .await
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Stream error: {}", e)))?
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Stream error: {}", e)))
            .boxed();
        Ok(stream)
    }
}
