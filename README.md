![runarchive-logo](assets/runarchive-logo.svg)

# runarchive

`runarchive` is a high-performance, asynchronous, zero-copy ZIP parser designed natively for both standard operating systems (Linux, macOS, Windows) and WebAssembly (WASM).

By treating remote archives (like those hosted on HTTP servers or S3 buckets) as random-access memory constructs via intelligent `Range` requests, `runarchive` drastically reduces bandwidth and memory footprints. It entirely bypasses the traditional requirement of downloading multi-gigabyte ZIP files to disk before extraction.

## Key Features

*   **Zero-Copy Metadata Parsing**: EOCD and Central Directory records are parsed by mapping structural views over fetched byte slices, completely avoiding heap allocations for metadata strings during traversal.
*   **Asynchronous HTTP Streaming**: Uses `reqwest` and native `tokio::fs` syscalls to selectively pull compressed byte ranges from remote URLs or local storage exclusively when a specific file is targeted for extraction.
*   **Intelligent Buffer Retention**: Resolves overlapping metadata seeks strictly inside a local cache, but seamlessly channels bulk data streaming directly to downstream decoders without intermediate buffering constraints.
*   **WebAssembly Ready**: Compiles natively to `wasm32-unknown-unknown` out of the box. Exposes a native JavaScript `Archive` class utilizing `wasm-bindgen`, transparently interacting directly with the browser's `fetch()` API to process remote archives directly within modern web applications.

## Supported Platforms

*   **Native OS (CLI / Library)**: Linux, Windows, macOS (`x86_64`, `aarch64`, etc.)
*   **Web Contexts**: Any modern web browser supporting WebAssembly (`wasm32-unknown-unknown`).

## Architecture

*   **`Archive` (Library)**: The core struct exposed through `runarchive::Archive`. It orchestrates `SeekerBox` wrappers.
*   **`FileSeeker` / `HttpSeeker`**: Native abstraction layers resolving asynchronous disk logic or HTTP range queries.
*   **WASM Bridge**: The WASM target dynamically drops `Send`/`Sync` thread boundaries to compile perfectly against single-threaded event loops.

## Usage Examples

### 1. Command Line Interface (CLI)

The CLI acts as a simple wrapper to extract or index ZIPs:

```bash
# List files in a local archive
cargo run -- tests/fixtures/simple_archive_no_comment.zip

# List files inside a remote HTTP archive seamlessly without downloading it
cargo run -- https://example.com/huge-dataset.zip

# Extract a specific file
cargo run -- https://example.com/huge-dataset.zip dataset.csv
```

### 2. Rust Native Library

```rust
use runarchive::Archive;

#[tokio::main]
async fn main() {
    let url = "https://example.com/dataset.zip";
    
    // Initialize the Archive dynamically (automatically resolves protocol)
    let mut archive = Archive::open(url).await.unwrap();
    
    // List available files
    for file in archive.list_files() {
        println!("Found: {} ({} bytes)", file.name, file.size);
    }
    
    // Stream remote extraction entirely zero-copy to an asynchronous output
    let mut stdout = tokio::io::stdout();
    archive.extract_file("dataset.csv", &mut stdout).await.unwrap();
}
```

### 3. JavaScript (WebAssembly / Browser)

Because the project leverages `wasm-bindgen`, you can compile the library specifically for web frontends.

**Compile to WASM:**
```bash
cargo build --target wasm32-unknown-unknown
wasm-pack build --target web
```

**JavaScript Integration:**
```javascript
import init, { Archive } from './pkg/runarchive.js';

async function run() {
    // Initialize WASM WebAssembly Engine
    await init();
    
    // Mount the archive directly targeting the remote HTTP server (bypassing the Node/Backend layer completely)
    const archive = await Archive.open("https://example.com/massive-archive.zip");
    
    // Query contents instantly
    const fileList = archive.listFiles();
    console.log(fileList);
    
    // Extract a targeted file resolving in a contiguous typed bytes array 
    const extractedBytes = await archive.extractFile("picture.jpg");
    
    // Example: Render the extracted bytes in the browser dynamically!
    const blob = new Blob([extractedBytes], { type: "image/jpeg" });
    document.getElementById('img').src = URL.createObjectURL(blob);
}
run();
```
