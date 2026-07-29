use bytes::Bytes;
use std::{
    io::{Error as IoError, ErrorKind},
    ops::RangeBounds,
    pin::Pin,
};

use futures::TryStreamExt;
use opendal::Reader as OpendalAsyncReader;
use url::Url;

pub fn split_zip_uri(uri: &str) -> Option<(String, String)> {
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

pub type ByteStream = Pin<Box<dyn futures::Stream<Item = Result<Bytes, IoError>> + Send>>;

pub struct Reader {
    inner: OpendalAsyncReader,
    file_size: u64,
}

impl Reader {
    pub async fn open(uri: &str) -> Result<Self, IoError> {
        let (uri, path) = split_zip_uri(uri)
            .ok_or_else(|| IoError::new(ErrorKind::InvalidInput, "Invalid ZIP URI"))?;

        // Reads resume in place after a dropped connection — see `crate::operator`.
        let operator = crate::operator::build(uri.as_str())?;

        let meta = operator
            .stat(&path)
            .await
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Failed to stat file: {}", e)))?;

        let file_size = meta.content_length();

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

    pub async fn read(&self, range: impl RangeBounds<u64>) -> Result<Bytes, IoError> {
        self.inner
            .read(range)
            .await
            .map(|b| b.to_bytes())
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Read error: {}", e)))
    }

    pub async fn read_to_stream(
        &self,
        range: impl RangeBounds<u64>,
    ) -> Result<ByteStream, IoError> {
        let stream = self
            .inner
            .clone()
            .into_bytes_stream(range)
            .await
            .map_err(|e| IoError::new(ErrorKind::Other, format!("Stream error: {}", e)))?;
        Ok(Box::pin(stream.map_err(|e| {
            IoError::new(ErrorKind::Other, format!("Stream error: {}", e))
        })))
    }

    #[cfg(feature = "async-futures")]
    pub async fn into_async_bufread(
        &self,
        range: impl RangeBounds<u64>,
    ) -> Result<impl futures::AsyncBufRead + Send + 'static, IoError> {
        let reader = self.inner.clone();
        let async_bufread = reader.into_futures_async_read(range).await.map_err(|e| {
            IoError::new(ErrorKind::Other, format!("Into async bufread error: {}", e))
        })?;
        Ok(async_bufread)
    }
}
