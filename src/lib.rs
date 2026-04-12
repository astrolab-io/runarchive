pub mod buffer;
pub mod parser;
pub mod seeker;
pub mod archive;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod wasm;

pub use archive::{Archive, FileEntry};
