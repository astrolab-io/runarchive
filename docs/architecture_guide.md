# Architectural Guide: Splitting the CLI & Exposing a WASM Module

Currently, `runarchive` operates as a mixed crate where `src/main.rs` contains all the orchestration logic (fetching the EOCD, traversing the Central Directory, and streaming the data) and `src/lib.rs` simply exposes the underlying modules.

To properly split the library from the CLI and compile it as a WASM module, you need two fundamental changes: 

1. **Move Orchestration to `lib.rs`**: The logic in `main.rs` that fetches the `RetentionBuffer` and iterates headers should be formalized into high-level API functions in `lib.rs`.
2. **Expose WASM Bindings**: We need to use `wasm-bindgen` to export these high-level Rust functions to JavaScript.

## 1. Splitting the CLI and Library

First, we need to create a primary struct in the library, such as `RunArchive`. This struct will encapsulate the `RetentionBuffer` and offer public methods.

### In `src/lib.rs` (The Library):
```rust
use crate::buffer::RetentionBuffer;
use crate::seeker::Seeker;
use std::io::Error as IoError;

pub struct Archive {
    buffer: RetentionBuffer<Box<dyn Seeker + Send + Sync + Unpin>>,
    // ... cache central directory headers if needed
}

impl Archive {
    /// Connects to a seeker, finds EOCD, and prepares the archive for reading
    pub async fn new(seeker: Box<dyn Seeker + Send + Sync + Unpin>, size: u64) -> Result<Self, IoError> {
        let mut buffer = RetentionBuffer::new(seeker, size).await?;
        
        let max_eocd_size = 65557;
        let fetch_start = size.saturating_sub(max_eocd_size);
        let fetch_len = (size - fetch_start) as usize;
        
        buffer.fetch_chunk(fetch_start, fetch_len).await?;
        
        // ... find and parse EOCD and Central Directory Headers here ...
        
        Ok(Self { buffer })
    }

    /// Returns a list of file names in the archive
    pub fn list_files(&self) -> Vec<String> {
        // ... return names from parsed headers ...
        vec![]
    }

    /// Extracts a specific file by name
    pub async fn extract_file<W: tokio::io::AsyncWrite + Unpin>(&mut self, filename: &str, mut writer: W) -> Result<(), IoError> {
        // ... copy the logic from main.rs that writes to stdout, 
        // but now it writes to 'writer' (which can be a file, stdout, or an in-memory buffer) ...
        Ok(())
    }
}
```

### In `src/main.rs` (The CLI Binary):
The CLI simply parses arguments and calls the library.

```rust
use runarchive::Archive;
use runarchive::seeker::{file::FileSeeker, http::HttpSeeker, Seeker};
use clap::Parser;

// ... CLI struct definition ...

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    // ... seeker resolution logic ...
    
    // Now just use the library!
    let mut archive = Archive::new(seeker, file_size).await.unwrap();
    
    if let Some(filename) = cli.filename {
        let mut stdout = tokio::io::stdout();
        archive.extract_file(&filename, &mut stdout).await.unwrap();
    } else {
        let files = archive.list_files();
        for file in files {
            println!("{}", file);
        }
    }
}
```

## 2. Compiling and Using as a WASM Module

To use this library in a browser environment, we must tackle two things:
1. `tokio::fs` and `reqwest` don't work natively in the browser. 
2. We need Javascript bindings.

### Adjusting `Cargo.toml`
The current `Cargo.toml` already isolates `tokio`/`reqwest` with `cfg(not(target_arch = "wasm32"))`.  We need to add `wasm-bindgen` and a new HTTP fetcher for the browser.

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["Window", "Request", "RequestInit", "Response", "Headers"] }
wasm-bindgen-futures = "0.4"
```

### Exposing Javascript Functions
In `src/lib.rs`, we define a WebAssembly boundary.

```rust
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct WasmArchive {
    // Inner architecture holding WASM-compatible seekers
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl WasmArchive {
    #[wasm_bindgen(constructor)]
    pub async fn open(url: String) -> Result<WasmArchive, JsValue> {
        // Implement a custom WasmHttpSeeker that uses Javascript's fetch() API 
        // under the hood via web_sys, since reqwest won't work.
        // ...
        Ok(WasmArchive { /* ... */ })
    }

    #[wasm_bindgen]
    pub fn list_files(&self) -> js_sys::Array {
        let array = js_sys::Array::new();
        // ... Push file names to the JS array
        array
    }

    #[wasm_bindgen]
    pub async fn extract_file(&mut self, filename: String) -> Result<js_sys::Uint8Array, JsValue> {
        // Extract the bytes and return them to Javascript as a typed array
        // so the browser can save it as an ObjectURL or Blob!
        // ...
        Ok(js_sys::Uint8Array::new_with_length(0))
    }
}
```

### Using the WASM Module in Javascript

Once compiled with `wasm-pack build --target web`, you can import it directly into a modern web app:

```html
<script type="module">
    import init, { Archive } from './pkg/runarchive.js';

    async function run() {
        await init();
        
        // Create the WASM struct pointing directly to an HTTP server
        const archive = await Archive.open("https://example.com/huge_archive.zip");
        
        // List files without downloading the zip
        const files = archive.listFiles();
        console.log("Files:", files);
        
        // Extract a specific file into a Javascript Uint8Array in memory
        const bytes = await archive.extractFile("test.txt");
        
        // Convert to a downloadable Blob
        const blob = new Blob([bytes]);
        const url = URL.createObjectURL(blob);
        window.open(url);
    }
    
    run();
</script>
```
