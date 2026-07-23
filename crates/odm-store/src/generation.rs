use odm_ir::Hash;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GenerationId(pub u64);

/// A consistent snapshot of the project sources: path → content hash of every
/// source file. Builds run against exactly one generation; results never mix
/// code versions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Generation {
    pub id: GenerationId,
    /// Project-relative path → blake3 of file contents. BTreeMap so the
    /// generation itself hashes canonically.
    pub sources: BTreeMap<String, Hash>,
    /// GC roots published for this generation (e.g. the built scene hash).
    pub roots: Vec<Hash>,
}

impl Generation {
    /// Hash identifying the source snapshot (independent of id/roots).
    pub fn sources_hash(&self) -> Hash {
        let mut w = odm_ir::Hasher::new();
        w.len(self.sources.len());
        for (path, h) in &self.sources {
            w.str(path);
            w.hash(h);
        }
        w.finish()
    }
}
