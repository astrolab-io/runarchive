pub mod archive;
pub mod parser;
pub mod reader;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use archive::{Archive, FileEntry};
