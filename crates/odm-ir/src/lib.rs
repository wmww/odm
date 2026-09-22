//! ODM IR: the contract between JS parts and the engine.
//!
//! Everything here is content-addressable via a canonical byte encoding fed to
//! blake3. Hashing is bit-exact (floats hashed as raw IEEE bits, no
//! canonicalization) so equal hashes imply byte-equal values.

mod canon;
mod hash;
mod types;

pub use canon::{Canonical, Hasher};
pub use hash::Hash;
pub use types::{Color, Mesh, MeshError, Node, Transform};

use serde_json::Value;

/// Canonical hash of a JSON value (map keys sorted; numbers as f64 bits).
/// Used for args / context-value hashing.
pub fn hash_json(v: &Value) -> Hash {
    Canonical::hash(v)
}
