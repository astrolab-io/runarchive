use clap::Parser;
use runarchive::buffer::RetentionBuffer;
use runarchive::parser;
use runarchive::seeker::{file::FileSeeker, http::HttpSeeker, Seeker};
use std::process::exit;

#[derive(Parser, Debug)]
#[command(
    name = "runarchive",
    version = "0.1",
    about = "Zero-copy async zip parser"
)]
struct Cli {
    #[arg(value_name = "URI")]
    uri: String,

    #[arg(value_name = "FILENAME")]
    filename: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Choose the seeker strategy based on URI
    let is_http = cli.uri.starts_with("http://") || cli.uri.starts_with("https://");

    let (seeker, file_size) = if is_http {
        let s = HttpSeeker::new(&cli.uri).await.unwrap_or_else(|e| {
            eprintln!("Failed to connect to URL: {}", e);
            exit(1);
        });
        let size = s.file_size();
        // Here we box the seeker to create a generic underlying type,
        // or for simplicity, we use dynamic dispatch
        (Box::new(s) as Box<dyn Seeker + Send + Sync + Unpin>, size)
    } else {
        let file_path = if let Some(stripped) = cli.uri.strip_prefix("file://") {
            stripped.to_string()
        } else {
            cli.uri.clone()
        };

        let s = FileSeeker::new(&file_path).await.unwrap_or_else(|e| {
            eprintln!("Failed to open local file: {}", e);
            exit(1);
        });
        // We need the file size for local files
        let metadata = tokio::fs::metadata(&file_path).await.unwrap();
        let size = metadata.len();
        (Box::new(s) as Box<dyn Seeker + Send + Sync + Unpin>, size)
    };

    let mut buffer = RetentionBuffer::new(seeker, file_size).await.unwrap();

    // Fetch the last 65557 bytes or the whole file
    let max_eocd_size = 65557;
    let fetch_start = if file_size > max_eocd_size {
        file_size - max_eocd_size
    } else {
        0
    };
    let fetch_len = (file_size - fetch_start) as usize;

    buffer.fetch_chunk(fetch_start, fetch_len).await.unwrap();

    // Scan for EOCD
    let slice = buffer.get_retained_slice(fetch_start, fetch_len).unwrap();
    let eocd_rel_offset = match parser::find_eocd_offset(slice) {
        Ok(idx) => idx,
        Err(_) => {
            eprintln!("Failed to find EOCD signature.");
            exit(1);
        }
    };

    let eocd = parser::parse_eocd(&slice[eocd_rel_offset..]).unwrap();

    // Fetch Central Directory
    buffer
        .fetch_chunk(eocd.cd_offset as u64, eocd.cd_size as usize)
        .await
        .unwrap();
    let cd_slice = buffer
        .get_retained_slice(eocd.cd_offset as u64, eocd.cd_size as usize)
        .unwrap();

    let headers = parser::iterate_central_directory(cd_slice, eocd.total_cd_records).unwrap();

    if let Some(filename) = cli.filename {
        let target = headers.iter().find(|h| h.file_name == filename);
        let (target_offset, target_method, target_size) = match target {
            Some(h) => (
                h.local_header_offset as u64,
                h.compression_method,
                h.compressed_size as u64,
            ),
            None => {
                eprintln!("File not found in archive");
                exit(1);
            }
        };

        drop(headers);

        use tokio::io::AsyncWriteExt;

        buffer
            .seek(std::io::SeekFrom::Start(target_offset))
            .await
            .unwrap();

        let mut lfh_fixed = vec![0u8; 30];
        buffer.read(&mut lfh_fixed).await.unwrap();

        if &lfh_fixed[0..4] != &[0x50, 0x4b, 0x03, 0x04] {
            eprintln!("Invalid Local File Header signature.");
            exit(1);
        }

        let fn_len = u16::from_le_bytes([lfh_fixed[26], lfh_fixed[27]]) as usize;
        let extra_len = u16::from_le_bytes([lfh_fixed[28], lfh_fixed[29]]) as usize;

        let payload_offset = target_offset + 30 + fn_len as u64 + extra_len as u64;
        buffer
            .seek(std::io::SeekFrom::Start(payload_offset))
            .await
            .unwrap();

        let mut stdout = tokio::io::stdout();

        if target_method == 0 {
            let mut remaining = target_size;
            let mut chunk = vec![0u8; 65536];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = buffer.read(&mut chunk[..to_read]).await.unwrap();
                if n == 0 {
                    break;
                }
                stdout.write_all(&chunk[..n]).await.unwrap();
                remaining -= n as u64;
            }
        } else if target_method == 8 {
            let mut decoder = async_compression::tokio::write::DeflateDecoder::new(stdout);
            let mut remaining = target_size;
            let mut chunk = vec![0u8; 65536];
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, chunk.len() as u64) as usize;
                let n = buffer.read(&mut chunk[..to_read]).await.unwrap();
                if n == 0 {
                    break;
                }
                decoder.write_all(&chunk[..n]).await.unwrap();
                remaining -= n as u64;
            }
            decoder.shutdown().await.unwrap();
        } else {
            eprintln!("Unsupported compression method: {}", target_method);
            exit(1);
        }
    } else {
        for h in headers {
            let name = h.file_name;
            println!("{} - {} bytes", name, h.uncompressed_size);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Dummy main for WASM native builds.
    // WASM leverages lib.rs entrypoints typically instead of a direct binary orchestrator
}
