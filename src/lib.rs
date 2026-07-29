pub mod parser;

/// Storage-handle construction shared by the blocking and async readers — this
/// is where the retry/resume policy for dropped connections lives.
#[cfg(any(feature = "sync", feature = "async-futures", feature = "async-tokio"))]
mod operator;

#[cfg(feature = "sync")]
pub mod blocking;

#[cfg(any(feature = "async-futures", feature = "async-tokio"))]
pub mod reader;

#[cfg(any(feature = "async-futures", feature = "async-tokio"))]
pub mod archive;

#[cfg(feature = "sync")]
pub use blocking::archive::{Archive, FileEntry};

#[cfg(any(feature = "async-futures", feature = "async-tokio"))]
#[cfg(not(feature = "sync"))]
pub use archive::{Archive, FileEntry};

#[cfg(target_arch = "wasm32")]
pub mod wasm;