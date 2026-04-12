pub mod buffer;
pub mod parser;
pub mod seeker;
pub mod archive;
pub mod deflate;

#[cfg(all(feature = "resume", not(target_os = "wasi")))]
pub mod resume;

#[cfg(feature = "cli")]
pub mod progress;

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
pub mod wasm;

pub use archive::{Archive, FileEntry};
