//! Content-addressed store: IR objects, build generations, memo cache.
//!
//! Thread-safe; shared across build workers, the CLI server, and the viewer
//! via `Arc<Store>`. In-memory for now — the API is designed so a disk-backed
//! layer can be added behind it later.

mod generation;
mod memo;
mod store;

pub use generation::{Generation, GenerationId};
pub use memo::{Dep, InvokeOutcome, LogLevel, LogLine, MemoEntry, MemoKey};
pub use store::{MEMO_CAP, MEMO_PER_KEY, Object, RootPin, Store};
