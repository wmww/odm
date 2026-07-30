use odm_ir::Hash;
use serde::{Deserialize, Serialize};

/// Memo lookup key. Dependencies are deliberately NOT part of the key — they
/// are only known after running — so entries store recorded deps which the
/// scheduler validates Salsa-style on hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoKey {
    /// Hash of the doohickey's source code.
    pub code: Hash,
    /// Canonical hash of the build args.
    pub args: Hash,
}

/// A recorded dependency of a build. Engine queries against content-addressed
/// inputs (raycasts, bounds, ...) are pure functions of their key and are not
/// recorded: same input hash → same result, forever.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Dep {
    /// A cascade input read from the build's environment. Valid while the
    /// value in the reader's environment hashes the same.
    Context { key: String, value: Hash },
    /// A nested `invoke(path, args, provides)`. Valid while `path` resolves
    /// to the same build output; validated recursively by the scheduler
    /// (which needs the actual args/provides values to re-run the invoked
    /// build if its memo is stale).
    Invoke {
        path: String,
        args: serde_json::Value,
        provides: serde_json::Map<String, serde_json::Value>,
        output: Hash,
    },
}

/// One captured console line from a build. Lives here (not odm-js) so memo
/// entries can replay logs on a hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoEntry {
    pub deps: Vec<Dep>,
    /// Hash of the build output object in the store.
    pub output: Hash,
    /// Console output of the original run, replayed on memo hits so
    /// `console.log` doesn't vanish when nothing changed.
    pub logs: Vec<LogLine>,
}
