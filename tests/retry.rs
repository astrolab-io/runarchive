//! A dropped response body must not cost the caller the bytes it already got.
//!
//! The failure this guards against: an origin closes the connection partway
//! through a long ranged read. Nothing above the reader can recover from that
//! (a DEFLATE stream has no mid-stream entry point), so the read itself has to
//! reconnect and continue from the byte it stopped on.
//!
//! wiremock cannot truncate a body mid-flight, so these tests drive a raw TCP
//! origin that answers `HEAD` with a size and then breaks its first ranged
//! `GET` after a known number of bytes.

#![cfg(all(feature = "sync", not(target_arch = "wasm32")))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A single-file HTTP origin: `HEAD` reports the size, `GET` serves the
/// requested range — except the first `GET`, which stops early and hangs up.
struct Origin {
    base_uri: String,
    /// Every `Range` header value received, in order.
    ranges: Arc<Mutex<Vec<String>>>,
}

async fn spawn_origin(body: Arc<Vec<u8>>, break_first_get_after: usize) -> Origin {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let ranges = Arc::new(Mutex::new(Vec::new()));

    let gets = Arc::new(AtomicUsize::new(0));
    let task_ranges = ranges.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            let ranges = task_ranges.clone();
            let gets = gets.clone();
            tokio::spawn(async move {
                let _ = serve(stream, body, ranges, gets, break_first_get_after).await;
            });
        }
    });

    Origin {
        base_uri: format!("http://{addr}"),
        ranges,
    }
}

async fn serve(
    mut stream: TcpStream,
    body: Arc<Vec<u8>>,
    ranges: Arc<Mutex<Vec<String>>>,
    gets: Arc<AtomicUsize>,
    break_first_get_after: usize,
) -> std::io::Result<()> {
    let head = read_request_head(&mut stream).await?;
    let total = body.len();

    // `Connection: close` everywhere: one request per connection keeps the
    // retry's request sequence unambiguous.
    if head.starts_with("HEAD") {
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\
             Connection: close\r\n\r\n"
        );
        stream.write_all(resp.as_bytes()).await?;
        return stream.shutdown().await;
    }

    let range = header_value(&head, "range");
    if let Some(value) = &range {
        ranges.lock().expect("ranges lock").push(value.clone());
    }
    let (start, end) = match &range {
        Some(value) => parse_byte_range(value, total),
        None => (0, total.saturating_sub(1)),
    };
    let slice = &body[start..=end];

    let status = if range.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    let mut resp = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\
         Connection: close\r\n",
        slice.len()
    );
    if range.is_some() {
        resp.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    resp.push_str("\r\n");
    stream.write_all(resp.as_bytes()).await?;

    // The declared Content-Length is honest; the body is not. Writing fewer
    // bytes and closing is what a dropped connection looks like to the client:
    // "error decoding response body", which opendal classifies as temporary.
    if gets.fetch_add(1, Ordering::SeqCst) == 0 && break_first_get_after < slice.len() {
        stream.write_all(&slice[..break_first_get_after]).await?;
    } else {
        stream.write_all(slice).await?;
    }
    stream.shutdown().await
}

async fn read_request_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await? == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_string())
}

/// `bytes=start-end`, either side optionally absent.
fn parse_byte_range(value: &str, total: usize) -> (usize, usize) {
    let spec = value.trim_start_matches("bytes=");
    let (start, end) = spec.split_once('-').unwrap_or((spec, ""));
    let start = start.parse().unwrap_or(0);
    let end = end.parse().unwrap_or(total.saturating_sub(1));
    (start, end.min(total.saturating_sub(1)))
}

const TOTAL: usize = 64 * 1024;
const BREAK_AT: usize = 32 * 1024;

fn payload() -> Arc<Vec<u8>> {
    // Position-dependent bytes, so a resume that lands on the wrong offset
    // shows up as a content mismatch rather than a length mismatch.
    Arc::new((0..TOTAL).map(|i| (i % 251) as u8).collect())
}

#[tokio::test(flavor = "multi_thread")]
async fn dropped_body_resumes_from_the_bytes_already_delivered() {
    let body = payload();
    let origin = spawn_origin(body.clone(), BREAK_AT).await;
    let uri = format!("{}/data.bin", origin.base_uri);

    // spawn_blocking: the blocking reader needs an ambient tokio handle, which
    // is exactly how callers use it.
    let got = tokio::task::spawn_blocking(move || {
        let reader = runarchive::blocking::reader::Reader::open(&uri)?;
        assert_eq!(reader.file_size(), TOTAL as u64);
        reader.read(0..TOTAL as u64)
    })
    .await
    .expect("join")
    .expect("a dropped body must be retried, not surfaced");

    assert_eq!(got.len(), TOTAL, "short read");
    assert_eq!(got.as_ref(), body.as_slice(), "resumed bytes are not contiguous");

    let ranges = origin.ranges.lock().expect("ranges lock").clone();
    assert_eq!(ranges.len(), 2, "expected one drop + one resume, saw {ranges:?}");
    assert_eq!(ranges[0], format!("bytes=0-{}", TOTAL - 1));
    // The retry asks only for what never arrived — the first half is not refetched.
    assert_eq!(ranges[1], format!("bytes={BREAK_AT}-{}", TOTAL - 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn intact_body_is_read_in_one_request() {
    let body = payload();
    // `TOTAL` is never < the slice length, so no GET is broken.
    let origin = spawn_origin(body.clone(), TOTAL).await;
    let uri = format!("{}/data.bin", origin.base_uri);

    let got = tokio::task::spawn_blocking(move || {
        runarchive::blocking::reader::Reader::open(&uri)?.read(0..TOTAL as u64)
    })
    .await
    .expect("join")
    .expect("read");

    assert_eq!(got.as_ref(), body.as_slice());
    let ranges = origin.ranges.lock().expect("ranges lock").clone();
    assert_eq!(ranges.len(), 1, "healthy read must not retry: {ranges:?}");
}
