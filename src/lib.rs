pub mod buffer;
pub mod parser;
pub mod seeker;
pub mod archive;
pub mod deflate;
pub mod progress;
pub mod resume;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use archive::{Archive, FileEntry};
