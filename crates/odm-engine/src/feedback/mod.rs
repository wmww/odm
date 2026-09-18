//! Bug reports and feature requests, from the agent or the user. Nothing
//! leaves the machine until a human presses Send on the viewer's Feedback
//! page: `item.rs` is the file that waits, `sink.rs` is where it goes.

pub mod item;
pub mod sink;

pub use item::{Item, build_string};
