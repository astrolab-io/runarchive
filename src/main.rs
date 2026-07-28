#[cfg(not(target_arch = "wasm32"))]
use clap::Parser;

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(all(not(target_arch = "wasm32"), feature = "async-tokio"))]
#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let mut archive = runarchive::Archive::open(&cli.uri)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to read archive metadata: {}", e);
            std::process::exit(1);
        });

    if let Some(filename) = cli.filename {
        let mut stdout = tokio::io::stdout();
        let mut reader = match archive.extract_file(&filename).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to extract file: {}", e);
                std::process::exit(1);
            }
        };
        if let Err(e) = tokio::io::copy(&mut reader, &mut stdout).await {
            eprintln!("Failed to write to stdout: {}", e);
            std::process::exit(1);
        }
    } else {
        let files = archive.list_files();
        for f in files {
            println!("{} - {} bytes", f.name, f.size);
        }
    }
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "async-futures",
    not(feature = "async-tokio")
))]
#[tokio::main]
async fn main() {
    use std::io::Write;
    let cli = Cli::parse();

    let mut archive = runarchive::Archive::open(&cli.uri)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to read archive metadata: {}", e);
            std::process::exit(1);
        });

    if let Some(filename) = cli.filename {
        let mut stdout = Vec::new();
        let mut reader = match archive.extract_file(&filename).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to extract file: {}", e);
                std::process::exit(1);
            }
        };
        futures::io::copy(&mut reader, &mut stdout)
            .await
            .unwrap_or(0);
        std::io::stdout().write_all(&stdout).unwrap();
    } else {
        let files = archive.list_files();
        for f in files {
            println!("{} - {} bytes", f.name, f.size);
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "sync"))]
fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let cli = Cli::parse();

    let mut archive = runarchive::Archive::open(&cli.uri).unwrap_or_else(|e| {
        eprintln!("Failed to read archive metadata: {}", e);
        std::process::exit(1);
    });

    if let Some(filename) = cli.filename {
        let mut stdout = std::io::stdout();
        let mut reader = match archive.extract_file(&filename) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to extract file: {}", e);
                std::process::exit(1);
            }
        };
        if let Err(e) = std::io::copy(&mut reader, &mut stdout) {
            eprintln!("Failed to write to stdout: {}", e);
            std::process::exit(1);
        }
    } else {
        let files = archive.list_files();
        for f in files {
            println!("{} - {} bytes", f.name, f.size);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Dummy main for WASM native builds.
    // WASM leverages lib.rs entrypoints typically instead of a direct binary orchestrator
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(feature = "sync", feature = "async-tokio", feature = "async-futures"))
))]
fn main() {
    eprintln!(
        "runarchive: the CLI requires one of the features `sync`, `async-tokio` or `async-futures`"
    );
    std::process::exit(1);
}
