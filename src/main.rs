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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(target_arch = "wasm32")]
fn main() {
    // Dummy main for WASM native builds.
    // WASM leverages lib.rs entrypoints typically instead of a direct binary orchestrator
}
