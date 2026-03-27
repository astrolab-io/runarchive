use crate::buffer::RetentionBuffer;
use crate::parser;
use crate::seeker::{Seeker, SeekerBox};
use std::io::{Error as IoError, ErrorKind};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub compression_method: u16,
    pub offset: u64,
}

pub struct Archive {
    uri: String,
    buffer: RetentionBuffer<SeekerBox>,
    entries: Vec<FileEntry>,
}

impl Archive {
    pub async fn open(uri: &str) -> Result<Self, IoError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let is_http = uri.starts_with("http://") || uri.starts_with("https://");

            let (seeker, file_size) = if is_http {
                let s = crate::seeker::http::HttpSeeker::new(uri).await?;
                let size = s.file_size();
                (Box::new(s) as SeekerBox, size)
            } else {
                let file_path = if let Some(stripped) = uri.strip_prefix("file://") {
                    stripped.to_string()
                } else {
                    uri.to_string()
                };

                let s = crate::seeker::file::FileSeeker::new(&file_path).await?;
                let metadata = tokio::fs::metadata(&file_path).await?;
                let size = metadata.len();
                (Box::new(s) as SeekerBox, size)
            };

            Self::from_seeker(uri, seeker, file_size).await
        }
        #[cfg(target_arch = "wasm32")]
        {
            let s = crate::seeker::http::HttpSeeker::new(uri).await?;
            let size = s.file_size();
            let seeker = Box::new(s) as SeekerBox;
            Self::from_seeker(uri, seeker, size).await
        }
    }

    /// Internal initializer from an already resolved seeker
    async fn from_seeker(uri: &str, seeker: SeekerBox, file_size: u64) -> Result<Self, IoError> {
        let mut buffer = RetentionBuffer::new(seeker, file_size).await?;

        let max_eocd_size = 65557;
        let fetch_start = file_size.saturating_sub(max_eocd_size);
        let fetch_len = (file_size - fetch_start) as usize;

        buffer.fetch_chunk(fetch_start, fetch_len).await?;

        // Block to limit the immutable borrow of `buffer`
        let parsed_headers = {
            let slice = buffer
                .get_retained_slice(fetch_start, fetch_len)
                .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "Failed to get EOCD slice"))?;
            let eocd_rel_offset = parser::find_eocd_offset(slice).map_err(|_| {
                IoError::new(ErrorKind::InvalidData, "Failed to find EOCD signature.")
            })?;
            let eocd = parser::parse_eocd(&slice[eocd_rel_offset..])
                .map_err(|_| IoError::new(ErrorKind::InvalidData, "Failed to parse EOCD"))?;

            buffer
                .fetch_chunk(eocd.cd_offset as u64, eocd.cd_size as usize)
                .await?;
            let cd_slice = buffer
                .get_retained_slice(eocd.cd_offset as u64, eocd.cd_size as usize)
                .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "Failed to get CD slice"))?;

            // We parse and immediately convert to owned structures to release the borrow on the buffer
            // so we can use the buffer for data extraction later. Filenames are small, so allocating
            // them is cheap and avoids complex self-referential lifetimes.
            let headers = parser::iterate_central_directory(cd_slice, eocd.total_cd_records)
                .map_err(|_| {
                    IoError::new(ErrorKind::InvalidData, "Failed to parse central directory")
                })?;

            let mut entries = Vec::with_capacity(headers.len());
            for h in headers {
                entries.push(FileEntry {
                    name: h.file_name.to_string(),
                    size: h.uncompressed_size as u64,
                    compressed_size: h.compressed_size as u64,
                    compression_method: h.compression_method,
                    offset: h.local_header_offset as u64,
                });
            }
            entries
        };

        Ok(Self {
            uri: uri.to_string(),
            buffer,
            entries: parsed_headers,
        })
    }

    /// Returns a list of cached file entries in the archive
    pub fn list_files(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Extracts a specific file by name to the provided AsyncWrite stream
    pub async fn extract_file<W: tokio::io::AsyncWrite + Unpin>(
        &mut self,
        filename: &str,
        mut writer: W,
        resumable: bool,
        progress: bool,
    ) -> Result<(), IoError> {
        let target = self.entries.iter().find(|e| e.name == filename).cloned();
        let target =
            target.ok_or_else(|| IoError::new(ErrorKind::NotFound, "File not found in archive"))?;

        let mut pb_download = None;
        let mut pb_decompress = None;
        let mut mp_ref = None;

        if progress {
            use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
            let mp = MultiProgress::new();

            let label = if self.uri.starts_with("http") {
                "Download"
            } else {
                "Read"
            };

            let style = ProgressStyle::with_template(
                "{spinner:.green} {prefix:>10} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})"
            ).unwrap().progress_chars("=> ");

            let pbd = mp.add(ProgressBar::new(target.compressed_size));
            pbd.set_style(style.clone());
            pbd.set_prefix(label.to_string());
            pbd.enable_steady_tick(std::time::Duration::from_millis(100));
            pb_download = Some(pbd);

            let pbc = mp.add(ProgressBar::new(target.size));
            pbc.set_style(style);
            pbc.set_prefix("Decompress");
            pbc.enable_steady_tick(std::time::Duration::from_millis(100));
            pb_decompress = Some(pbc);

            mp_ref = Some(mp);
        }

        self.buffer
            .seek(std::io::SeekFrom::Start(target.offset))
            .await?;

        let mut lfh_fixed = vec![0u8; 30];
        self.buffer.read(&mut lfh_fixed).await?;

        if &lfh_fixed[0..4] != &[0x50, 0x4b, 0x03, 0x04] {
            return Err(IoError::new(
                ErrorKind::InvalidData,
                "Invalid Local File Header signature",
            ));
        }

        let fn_len = u16::from_le_bytes([lfh_fixed[26], lfh_fixed[27]]) as usize;
        let extra_len = u16::from_le_bytes([lfh_fixed[28], lfh_fixed[29]]) as usize;

        let payload_offset = target.offset + 30 + fn_len as u64 + extra_len as u64;
        self.buffer
            .seek(std::io::SeekFrom::Start(payload_offset))
            .await?;

        use tokio::io::AsyncWriteExt;

        if target.compression_method == 0 {
            let mut remaining = target.compressed_size;
            let mut chunk = vec![0u8; 65536];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = self.buffer.read(&mut chunk[..to_read]).await?;
                if n == 0 {
                    break;
                }
                writer.write_all(&chunk[..n]).await?;
                remaining -= n as u64;

                if let Some(ref pb) = pb_download {
                    pb.inc(n as u64);
                }
                if let Some(ref pb) = pb_decompress {
                    pb.inc(n as u64);
                }
            }
        } else if target.compression_method == 8 {
            use libz_sys::*;
            use std::ffi::c_int;

            struct ZStreamSend(z_stream);
            unsafe impl Send for ZStreamSend {}

            extern "C" {
                fn malloc(size: usize) -> *mut std::ffi::c_void;
                fn free(p: *mut std::ffi::c_void);
            }
            unsafe extern "C" fn zalloc_stub(
                _opaque: *mut std::ffi::c_void,
                items: std::ffi::c_uint,
                size: std::ffi::c_uint,
            ) -> *mut std::ffi::c_void {
                malloc((items * size) as usize)
            }
            unsafe extern "C" fn zfree_stub(
                _opaque: *mut std::ffi::c_void,
                ptr: *mut std::ffi::c_void,
            ) {
                free(ptr)
            }

            let checkpoint = if resumable {
                crate::resume::platform::load_checkpoint(&self.uri, filename).await
            } else {
                None
            };

            let mut stream_obj = ZStreamSend(z_stream {
                next_in: std::ptr::null_mut(),
                avail_in: 0,
                total_in: 0,
                next_out: std::ptr::null_mut(),
                avail_out: 0,
                total_out: 0,
                msg: std::ptr::null_mut(),
                state: std::ptr::null_mut(),
                zalloc: zalloc_stub,
                zfree: zfree_stub,
                opaque: std::ptr::null_mut(),
                data_type: 0,
                adler: 0,
                reserved: 0,
            });

            let version = b"1.3.0\0".as_ptr() as *const std::ffi::c_char;
            unsafe {
                let init_err = inflateInit2_(
                    &mut stream_obj.0,
                    -15,
                    version,
                    std::mem::size_of::<z_stream>() as c_int,
                );
                if init_err != Z_OK {
                    return Err(IoError::new(
                        ErrorKind::Other,
                        "Failed to initialize z_stream",
                    ));
                }
            }

            // Shared cursor updated on every DEFLATE block boundary.
            // The CheckpointGuard ensures it is saved to disk even if the process is killed
            // before the next periodic save — preventing overlapping bytes on append-resume.
            #[cfg(not(target_arch = "wasm32"))]
            let shared_cursor: std::sync::Arc<
                std::sync::Mutex<Option<crate::resume::ResumableCursor>>,
            > = std::sync::Arc::new(std::sync::Mutex::new(None));

            #[cfg(not(target_arch = "wasm32"))]
            let _guard = if resumable {
                Some(crate::resume::CheckpointGuard::new(
                    &self.uri,
                    filename,
                    std::sync::Arc::clone(&shared_cursor),
                ))
            } else {
                None
            };

            let mut current_payload_offset = payload_offset;
            let mut last_checkpoint_uncompressed = 0;
            let mut uncompressed_offset = 0;
            let mut dictionary_window = std::collections::VecDeque::with_capacity(32768);
            if let Some(cp) = checkpoint {
                unsafe {
                    let b = cp.bits;
                    if cp.bit_count > 0 {
                        let ret =
                            inflatePrime(&mut stream_obj.0, cp.bit_count as c_int, b as c_int);
                        if ret != Z_OK {
                            inflateEnd(&mut stream_obj.0);
                            return Err(IoError::new(
                                ErrorKind::Other,
                                format!("inflatePrime failed: {}", ret),
                            ));
                        }
                    }
                    if !cp.dictionary_window.is_empty() {
                        let ret = inflateSetDictionary(
                            &mut stream_obj.0,
                            cp.dictionary_window.as_ptr(),
                            cp.dictionary_window.len() as u32,
                        );
                        if ret != Z_OK {
                            inflateEnd(&mut stream_obj.0);
                            return Err(IoError::new(
                                ErrorKind::Other,
                                format!("inflateSetDictionary failed: {}", ret),
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
                if let Some(ref mp) = mp_ref {
                    mp.println(resume_msg).unwrap_or(());
                } else {
                    eprintln!("{}", resume_msg);
                }
            }

            self.buffer
                .seek(std::io::SeekFrom::Start(current_payload_offset))
                .await?;
            let interval = crate::resume::calculate_checkpoint_interval(target.size);

            let mut chunk = vec![0u8; 65536];
            let mut out_buffer = vec![0u8; 65536];
            let total_compressed_read = current_payload_offset - payload_offset;

            let mut remaining = target.compressed_size.saturating_sub(total_compressed_read);

            if let Some(ref pb) = pb_download {
                pb.set_position(total_compressed_read);
            }
            if let Some(ref pb) = pb_decompress {
                pb.set_position(uncompressed_offset);
            }

            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = self.buffer.read(&mut chunk[..to_read]).await?;
                if n == 0 {
                    break;
                }

                remaining -= n as u64;
                if let Some(ref pb) = pb_download {
                    pb.inc(n as u64);
                }

                stream_obj.0.next_in = chunk.as_mut_ptr();
                stream_obj.0.avail_in = n as u32;

                while stream_obj.0.avail_in > 0 {
                    stream_obj.0.next_out = out_buffer.as_mut_ptr();
                    stream_obj.0.avail_out = out_buffer.len() as u32;

                    let ret = unsafe { inflate(&mut stream_obj.0, Z_BLOCK) };

                    if ret != Z_OK && ret != Z_STREAM_END && ret != Z_BUF_ERROR {
                        unsafe {
                            inflateEnd(&mut stream_obj.0);
                        }
                        return Err(IoError::new(
                            ErrorKind::InvalidData,
                            format!("zlib inflate error: {}", ret),
                        ));
                    }

                    let written = out_buffer.len() - stream_obj.0.avail_out as usize;
                    if written > 0 {
                        writer.write_all(&out_buffer[..written]).await?;
                        uncompressed_offset += written as u64;
                        if let Some(ref pb) = pb_decompress {
                            pb.inc(written as u64);
                        }

                        for &b in &out_buffer[..written] {
                            if dictionary_window.len() == 32768 {
                                dictionary_window.pop_front();
                            }
                            dictionary_window.push_back(b);
                        }
                    }

                    // Check block boundary: data_type bit 7 = boundary, bit 6 = last block
                    let data_type = stream_obj.0.data_type;
                    if resumable && (data_type & 128 != 0 && data_type & 64 == 0) {
                        let bit_count = (data_type & 7) as u8;
                        let mut bits_value = 0u8;
                        let current_in_offset =
                            current_payload_offset + n as u64 - stream_obj.0.avail_in as u64;

                        if bit_count > 0 {
                            unsafe {
                                let last_byte_ptr = stream_obj.0.next_in.offset(-1);
                                let last_byte = *last_byte_ptr;
                                bits_value = last_byte >> (8 - bit_count);
                            }
                        }

                        let win_vec: Vec<u8> = dictionary_window.iter().copied().collect();

                        let latest = crate::resume::ResumableCursor {
                            compressed_offset: current_in_offset,
                            uncompressed_offset,
                            bit_count,
                            bits: bits_value,
                            dictionary_window: win_vec,
                        };

                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            if let Ok(mut lock) = shared_cursor.lock() {
                                *lock = Some(latest.clone());
                            }
                        }

                        if uncompressed_offset - last_checkpoint_uncompressed >= interval {
                            if let Err(e) = crate::resume::platform::save_checkpoint(
                                &self.uri, filename, &latest,
                            )
                            .await
                            {
                                eprintln!("Warning: Failed to save checkpoint: {}", e);
                            }
                            last_checkpoint_uncompressed = uncompressed_offset;
                        }
                    }

                    if ret == Z_STREAM_END {
                        break;
                    }
                }
                current_payload_offset += n as u64;
            }

            unsafe {
                inflateEnd(&mut stream_obj.0);
            }

            if let Some(pb) = pb_download {
                pb.finish_with_message("Done");
            }
            if let Some(pb) = pb_decompress {
                pb.finish_with_message("Done");
            }

            // Disarm the guard: on drop it will delete the checkpoint file instead of saving.
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(ref guard) = _guard {
                guard.disarm();
            }
        } else {
            return Err(IoError::new(
                ErrorKind::Unsupported,
                format!(
                    "Unsupported compression method: {}",
                    target.compression_method
                ),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::path::PathBuf;

    // ── Slow writer ───────────────────────────────────────────────────────────
    // Blocks the tokio worker for 10 ms per chunk so the extraction of a 20 MB
    // file takes ~3 s.  That gives the abort timer (1.5 s) a reliable window to
    // fire AFTER the first checkpoint (≈ 800 ms = 5 MB / 65 536 × 10 ms) but
    // BEFORE full completion.  The `multi_thread` scheduler lets the timer run
    // on a different worker while this one is inside `thread::sleep`.
    struct SlowVecWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl tokio::io::AsyncWrite for SlowVecWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.0.lock().unwrap().extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    /// Generates an 11 MB ZIP (all-zeros, DEFLATE level 6) using pure Rust.
    async fn generate_test_zip() -> PathBuf {
        let zip_path = env::temp_dir().join("test_dynamic_large_archive.zip");
        let zip_path_clone = zip_path.clone();

        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

            let file = std::fs::File::create(&zip_path_clone).unwrap();
            let mut zip = ZipWriter::new(file);
            let opts = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(6));

            zip.start_file("large_test_file.bin", opts).unwrap();

            // 11 MB of zeros — compresses well, exercises the basic extract path
            let zeros = vec![0u8; 11 * 1024 * 1024];
            zip.write_all(&zeros).unwrap();
            zip.finish().unwrap();
        })
        .await
        .unwrap();

        zip_path
    }

    /// Generates a 20 MB repeating-pattern ZIP (DEFLATE level 1) using pure Rust.
    ///
    /// The 0..=255 cycling pattern at level 1 produces many DEFLATE block
    /// boundaries, guaranteeing at least one checkpoint in a partial extraction
    /// at the configured 5 MB interval.
    async fn generate_resume_test_zip() -> (PathBuf, &'static str) {
        const FILENAME: &str = "resume_payload.bin";
        let zip_path = env::temp_dir().join("resume_integrity_test_archive.zip");
        let zip_path_clone = zip_path.clone();

        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

            let file = std::fs::File::create(&zip_path_clone).unwrap();
            let mut zip = ZipWriter::new(file);
            // Level 1 (fastest) maximises the number of DEFLATE block boundaries
            let opts = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(1));

            zip.start_file(FILENAME, opts).unwrap();
            // 20 MB of 0..=255 cycling — diverse enough that the compressor
            // emits many separate blocks rather than one monolithic stream
            let data: Vec<u8> = (0u8..=255).cycle().take(20 * 1024 * 1024).collect();
            zip.write_all(&data).unwrap();
            zip.finish().unwrap();
        })
        .await
        .unwrap();

        (zip_path, FILENAME)
    }

    #[tokio::test]
    async fn test_archive_open_and_extract() {
        let zip_path = generate_test_zip().await;
        let uri = format!("file://{}", zip_path.display());

        // Test Open
        let mut archive = Archive::open(&uri).await.expect("Failed to open archive");
        let files = archive.list_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "large_test_file.bin");
        assert_eq!(files[0].size, 11 * 1024 * 1024);
        assert_eq!(files[0].compression_method, 8); // DEFLATE

        // Test Extract
        let out_path = env::temp_dir().join("extracted_large_test_file.bin");
        let mut out_file = tokio::fs::File::create(&out_path).await.unwrap();

        archive
            .extract_file("large_test_file.bin", &mut out_file, false, false)
            .await
            .expect("Failed to extract file");

        // Let's verify size
        let metadata = tokio::fs::metadata(&out_path).await.unwrap();
        assert_eq!(metadata.len(), 11 * 1024 * 1024);

        // Cleanup
        let _ = tokio::fs::remove_file(&out_path).await;
        let _ = tokio::fs::remove_file(&zip_path).await;
    }

    #[tokio::test]
    async fn test_archive_extract_resumable() {
        let zip_path = generate_test_zip().await;
        let uri = format!("file://{}", zip_path.display());

        let mut archive = Archive::open(&uri).await.expect("Failed to open archive");

        let out_path = env::temp_dir().join("extracted_large_test_file_resumable.bin");
        let mut out_file = tokio::fs::File::create(&out_path).await.unwrap();

        // Extract with resumable tracking.
        archive
            .extract_file("large_test_file.bin", &mut out_file, true, false)
            .await
            .expect("Failed to extract file");

        let metadata = tokio::fs::metadata(&out_path).await.unwrap();
        assert_eq!(metadata.len(), 11 * 1024 * 1024);

        // Cleanup
        let _ = tokio::fs::remove_file(&out_path).await;
        let _ = tokio::fs::remove_file(&zip_path).await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_resume_interrupt_and_append_integrity() {
        let (zip_path, filename) = generate_resume_test_zip().await;
        let uri = format!("file://{}", zip_path.display());

        // 1. Get Ground Truth (single pass)
        let mut ground_truth = Vec::new();
        {
            struct SimpleWriter<'a>(&'a mut Vec<u8>);
            impl tokio::io::AsyncWrite for SimpleWriter<'_> {
                fn poll_write(
                    mut self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                    buf: &[u8],
                ) -> std::task::Poll<std::io::Result<usize>> {
                    self.0.extend_from_slice(buf);
                    std::task::Poll::Ready(Ok(buf.len()))
                }
                fn poll_flush(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<std::io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }
                fn poll_shutdown(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<std::io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }
            }

            let mut archive = Archive::open(&uri).await.unwrap();
            archive
                .extract_file(filename, SimpleWriter(&mut ground_truth), false, false)
                .await
                .unwrap();
        }

        // 2. Interrupted session using the specialized SlowVecWriter
        let partial_data = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = SlowVecWriter(std::sync::Arc::clone(&partial_data));

        {
            let uri_clone = uri.clone();
            let handle = tokio::spawn(async move {
                let mut archive = Archive::open(&uri_clone).await.unwrap();
                let _ = archive.extract_file(filename, writer, true, false).await;
            });

            // Wait for 1.5s — the first checkpoint (at ~5MB) should have triggered
            // after ~0.8s. The abort should happen mid-file.
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            handle.abort();
            let _ = handle.await; // Wait for the task to be dropped and guard to run.
        }

        // 3. Load checkpoint and check the alignment
        let checkpoint = crate::resume::platform::load_checkpoint(&uri, filename)
            .await
            .expect("Failure: A checkpoint should have been saved by the Drop guard on abort");

        // The partial output might contain more bytes than recorded in the checkpoint
        // because we abort mid-write, but the Guard saves
        // the last successfully completed DEFLATE block boundary's offset.
        let mut final_data = partial_data.lock().unwrap().clone();

        // Truncate to the "safe offset" recorded in checkpoint to avoid overlaps.
        final_data.truncate(checkpoint.uncompressed_offset as usize);

        // 4. Resume extraction from where we left off
        {
            struct SimpleWriter<'a>(&'a mut Vec<u8>);
            impl tokio::io::AsyncWrite for SimpleWriter<'_> {
                fn poll_write(
                    mut self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                    buf: &[u8],
                ) -> std::task::Poll<std::io::Result<usize>> {
                    self.0.extend_from_slice(buf);
                    std::task::Poll::Ready(Ok(buf.len()))
                }
                fn poll_flush(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<std::io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }
                fn poll_shutdown(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> std::task::Poll<std::io::Result<()>> {
                    std::task::Poll::Ready(Ok(()))
                }
            }

            let mut archive = Archive::open(&uri).await.unwrap();
            archive
                .extract_file(filename, SimpleWriter(&mut final_data), true, false)
                .await
                .unwrap();
        }

        // 5. Assert byte-perfect integrity
        assert_eq!(
            final_data.len(),
            ground_truth.len(),
            "Final uncompressed size mismatch"
        );
        assert_eq!(
            final_data, ground_truth,
            "Corruption detected: Byte integrity failure after resume"
        );

        // Cleanup
        crate::resume::platform::delete_checkpoint(&uri, filename).await;
        let _ = tokio::fs::remove_file(&zip_path).await;
    }
}
