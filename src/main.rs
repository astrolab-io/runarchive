#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use clap::Parser;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::process::exit;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
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

    let mut archive = runarchive::Archive::open(&cli.uri).await.unwrap_or_else(|e| {
        eprintln!("Failed to read archive metadata: {}", e);
        exit(1);
    });

    if let Some(filename) = cli.filename {
        let mut stdout = tokio::io::stdout();
        if let Err(e) = archive.extract_file(&filename, &mut stdout).await {
            eprintln!("Failed to extract file: {}", e);
            exit(1);
        }
    } else {
        let files = archive.list_files();
        for f in files {
            println!("{} - {} bytes", f.name, f.size);
        }
    }
}

#[cfg(target_os = "wasi")]
#[wstd::main]
async fn main() {
    let cli = Cli::parse();

    let mut archive = runarchive::Archive::open(&cli.uri).await.unwrap_or_else(|e| {
        eprintln!("Failed to read archive metadata: {}", e);
        exit(1);
    });

    if let Some(filename) = cli.filename {
        let mut stdout = wstd::io::stdout();
        if let Err(e) = archive.extract_file(&filename, &mut stdout).await {
            eprintln!("Failed to extract file: {}", e);
            exit(1);
        }
    } else {
        let files = archive.list_files();
        for f in files {
            println!("{} - {} bytes", f.name, f.size);
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn main() {
    // Dummy main for browser WASM builds.
}
