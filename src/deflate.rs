use std::io::{Error as IoError, ErrorKind, SeekFrom};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use tokio::io::{AsyncWrite, AsyncWriteExt};
use zlib_rs::c_api::z_stream;
use zlib_rs::inflate::{self, InflateConfig, InflateStream};
use zlib_rs::{InflateFlush, ReturnCode};

use crate::seeker::Seeker;

#[cfg(all(feature = "resume", not(target_os = "wasi")))]
use crate::resume::{platform, calculate_checkpoint_interval, CheckpointGuard, ResumableCursor};

/// A wrapper around z_stream to allow it to be sent between threads.
/// This is safe because we only access it from a single thread at a time
/// during the async decompression loop.
struct SendZStream(z_stream);
unsafe impl Send for SendZStream {}
unsafe impl Sync for SendZStream {}

impl std::ops::Deref for SendZStream {
    type Target = z_stream;
    fn deref(&self) -> &Self::Target { &self.0 }
}
impl std::ops::DerefMut for SendZStream {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 }
}

/// A trait for decouplable progress tracking.
// ...
// (rest of the code remains similar but using SendZStream)
pub trait ProgressObserver: Send + Sync {
    fn update_download(&self, bytes: u64);
    fn update_decompress(&self, bytes: u64);
    fn set_download_position(&self, bytes: u64);
    fn set_decompress_position(&self, bytes: u64);
    fn println(&self, msg: String);
    fn finish(&self);
}

pub struct DecompressionOptions<'a> {
    pub uri: &'a str,
    pub filename: &'a str,
    pub payload_offset: u64,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub resumable: bool,
}

/// Handles DEFLATE decompression with resumability support using ZRAN (Zlib Random Access).
///
/// Only available on non-WASM targets. For WASM use `decompress_deflate_callback`.
#[cfg(not(target_arch = "wasm32"))]
pub async fn decompress_deflate<S, W>(
    seeker: &mut S,
    mut writer: W,
    progress: Option<Arc<dyn ProgressObserver>>,
    options: DecompressionOptions<'_>,
) -> Result<(), IoError>
where
    S: Seeker + Unpin,
    W: AsyncWrite + Unpin,
{
    #[cfg(all(feature = "resume", not(target_os = "wasi")))]
    let checkpoint = if options.resumable {
        platform::load_checkpoint(options.uri, options.filename).await
    } else {
        None
    };

    let mut strm_wrapper = SendZStream(z_stream::default());
    let mut config = InflateConfig::default();
    config.window_bits = -15; // Raw deflate

    if inflate::init(&mut strm_wrapper.0, config) != ReturnCode::Ok {
        return Err(IoError::new(
            ErrorKind::Other,
            "Failed to initialize zlib stream",
        ));
    }

    // Shared cursor updated on every DEFLATE block boundary.
    #[cfg(all(feature = "resume", not(target_os = "wasi")))]
    let shared_cursor: Arc<Mutex<Option<ResumableCursor>>> =
        Arc::new(Mutex::new(None));

    #[cfg(all(feature = "resume", not(target_os = "wasi")))]
    let _guard = if options.resumable {
        Some(CheckpointGuard::new(
            options.uri,
            options.filename,
            Arc::clone(&shared_cursor),
        ))
    } else {
        None
    };

    let mut current_payload_offset = options.payload_offset;
    let mut last_checkpoint_uncompressed = 0;
    let mut uncompressed_offset = 0;
    let mut dictionary_window = VecDeque::with_capacity(32768);

    #[cfg(all(feature = "resume", not(target_os = "wasi")))]
    if let Some(cp) = checkpoint {
        {
            let stream = unsafe { InflateStream::from_stream_mut(&mut strm_wrapper.0).unwrap() };
            if cp.bit_count > 0 {
                let ret = inflate::prime(stream, cp.bit_count as i32, cp.bits as i32);
                if ret != ReturnCode::Ok {
                    return Err(IoError::new(
                        ErrorKind::Other,
                        format!("inflatePrime failed: {:?}", ret),
                    ));
                }
            }
            if !cp.dictionary_window.is_empty() {
                let ret = inflate::set_dictionary(stream, &cp.dictionary_window);
                if ret != ReturnCode::Ok {
                    return Err(IoError::new(
                        ErrorKind::Other,
                        format!("inflateSetDictionary failed: {:?}", ret),
                    ));
                }
            }
        }

        last_checkpoint_uncompressed = cp.uncompressed_offset;
        uncompressed_offset = cp.uncompressed_offset;
        for &b in &cp.dictionary_window {
            dictionary_window.push_back(b);
        }

        current_payload_offset = cp.compressed_offset;

        let resume_msg = format!(
            "=> Resume Checkpoint Loaded (ZRAN)\n=> Resuming from {} bytes.",
            uncompressed_offset
        );
        if let Some(ref p) = progress {
            p.println(resume_msg);
        }
    }

    seeker.seek(SeekFrom::Start(current_payload_offset)).await?;
    
    #[cfg(all(feature = "resume", not(target_os = "wasi")))]
    let interval = calculate_checkpoint_interval(options.uncompressed_size);
    #[cfg(any(not(feature = "resume"), target_os = "wasi"))]
    let interval = u64::MAX;

    let mut chunk = vec![0u8; 65536];
    let mut out_buffer = vec![0u8; 65536];
    let total_compressed_read_at_start = current_payload_offset - options.payload_offset;
    let mut remaining = options.compressed_size.saturating_sub(total_compressed_read_at_start);

    if let Some(ref p) = progress {
        p.set_download_position(total_compressed_read_at_start);
        p.set_decompress_position(uncompressed_offset);
    }

    while remaining > 0 {
        let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
        let n = seeker.read(&mut chunk[..to_read]).await?;
        if n == 0 {
            break;
        }

        remaining -= n as u64;
        if let Some(ref p) = progress {
            p.update_download(n as u64);
        }

        strm_wrapper.next_in = chunk.as_ptr();
        strm_wrapper.avail_in = n as u32;

        while strm_wrapper.avail_in > 0 {
            strm_wrapper.next_out = out_buffer.as_mut_ptr();
            strm_wrapper.avail_out = out_buffer.len() as u32;

            let ret = {
                let stream = unsafe { InflateStream::from_stream_mut(&mut strm_wrapper.0).unwrap() };
                unsafe { inflate::inflate(stream, InflateFlush::Block) }
            };

            if ret != ReturnCode::Ok && ret != ReturnCode::StreamEnd && ret != ReturnCode::BufError {
                return Err(IoError::new(
                    ErrorKind::InvalidData,
                    format!("zlib inflate error: {:?}", ret),
                ));
            }

            let written = out_buffer.len() - strm_wrapper.avail_out as usize;
            if written > 0 {
                writer.write_all(&out_buffer[..written]).await?;
                uncompressed_offset += written as u64;
                if let Some(ref p) = progress {
                    p.update_decompress(written as u64);
                }

                for &b in &out_buffer[..written] {
                    if dictionary_window.len() == 32768 {
                        dictionary_window.pop_front();
                    }
                    dictionary_window.push_back(b);
                }
            }

            // Periodic checkpoint and block boundary tracking.
            #[cfg(all(feature = "resume", not(target_os = "wasi")))]
            {
                let data_type = strm_wrapper.data_type;
                if options.resumable && (data_type & 128 != 0 && data_type & 64 == 0) {
                    let bit_count = (data_type & 63) as u8;
                    let mut bits_value = 0u8;
                    let current_in_offset = current_payload_offset + n as u64 - strm_wrapper.avail_in as u64;

                    if bit_count > 0 {
                        let last_byte_idx = n - strm_wrapper.avail_in as usize - 1;
                        if last_byte_idx < n {
                            let last_byte = chunk[last_byte_idx];
                            bits_value = last_byte >> (8 - bit_count);
                        }
                    }

                    let latest = ResumableCursor {
                        compressed_offset: current_in_offset,
                        uncompressed_offset,
                        bits: bits_value,
                        bit_count,
                        dictionary_window: dictionary_window.iter().cloned().collect(),
                    };

                    {
                        let mut lock = shared_cursor.lock().unwrap();
                        *lock = Some(latest.clone());
                    }

                    if uncompressed_offset - last_checkpoint_uncompressed >= interval {
                        if let Err(e) = platform::save_checkpoint(
                            options.uri, options.filename, &latest,
                        ).await {
                            eprintln!("Warning: Failed to save checkpoint: {}", e);
                        }
                        last_checkpoint_uncompressed = uncompressed_offset;
                    }
                }
            }

            if ret == ReturnCode::StreamEnd {
                remaining = 0;
                break;
            }
        }
        current_payload_offset += n as u64;
    }

    {
        let stream = unsafe { InflateStream::from_stream_mut(&mut strm_wrapper.0).unwrap() };
        inflate::end(stream);
    }

    if let Some(ref p) = progress {
        p.finish();
    }

    // Disarm the guard
    #[cfg(all(feature = "resume", not(target_os = "wasi")))]
    if let Some(guard) = _guard {
        guard.disarm();
    }

    Ok(())
}

/// DEFLATE decompressor that delivers output via a synchronous `FnMut` callback instead of
/// `tokio::io::AsyncWrite`. This is the WASM-compatible path: no `tokio`, no `Arc<Mutex<…>>`,
/// no checkpoint guards (resumability is disabled on WASM).
pub async fn decompress_deflate_callback<S, F>(
    seeker: &mut S,
    write_chunk: &mut F,
    options: DecompressionOptions<'_>,
) -> Result<(), IoError>
where
    S: Seeker + Unpin,
    F: FnMut(&[u8]) -> Result<(), IoError>,
{
    let mut strm_wrapper = SendZStream(z_stream::default());
    let mut config = InflateConfig::default();
    config.window_bits = -15; // Raw deflate

    if inflate::init(&mut strm_wrapper.0, config) != ReturnCode::Ok {
        return Err(IoError::new(
            ErrorKind::Other,
            "Failed to initialize zlib stream",
        ));
    }

    seeker
        .seek(SeekFrom::Start(options.payload_offset))
        .await?;

    let mut remaining = options.compressed_size;
    let mut chunk = vec![0u8; 65536];
    let mut out_buffer = vec![0u8; 65536];

    while remaining > 0 {
        let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
        let n = seeker.read(&mut chunk[..to_read]).await?;
        if n == 0 {
            break;
        }
        remaining -= n as u64;

        strm_wrapper.next_in = chunk.as_ptr();
        strm_wrapper.avail_in = n as u32;

        while strm_wrapper.avail_in > 0 {
            strm_wrapper.next_out = out_buffer.as_mut_ptr();
            strm_wrapper.avail_out = out_buffer.len() as u32;

            let ret = {
                let stream =
                    unsafe { InflateStream::from_stream_mut(&mut strm_wrapper.0).unwrap() };
                unsafe { inflate::inflate(stream, InflateFlush::Block) }
            };

            if ret != ReturnCode::Ok
                && ret != ReturnCode::StreamEnd
                && ret != ReturnCode::BufError
            {
                return Err(IoError::new(
                    ErrorKind::InvalidData,
                    format!("zlib inflate error: {:?}", ret),
                ));
            }

            let written = out_buffer.len() - strm_wrapper.avail_out as usize;
            if written > 0 {
                write_chunk(&out_buffer[..written])?;
            }

            if ret == ReturnCode::StreamEnd {
                remaining = 0;
                break;
            }
        }
    }

    {
        let stream = unsafe { InflateStream::from_stream_mut(&mut strm_wrapper.0).unwrap() };
        inflate::end(stream);
    }

    Ok(())
}
