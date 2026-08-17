use odm_ir::Hash;
use serde::{Deserialize, Serialize};

/// Memo lookup key. Dependencies are deliberately NOT part of the key — they
/// are only known after running — so entries store recorded deps which the
/// scheduler validates Salsa-style on hit. The store keeps a bounded MRU
/// list of entries per key (one per environment seen), so scrubbing a
/// cascade value like `t` back and forth revalidates instead of rebuilding.
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
    Cascade { key: String, value: Hash },
    /// A nested `invoke(path, args, cascade)`. Valid while `path` resolves
    /// to the same outcome; validated recursively by the scheduler
    /// (which needs the actual args/cascade values to re-run the invoked
    /// build if its memo is stale).
    Invoke {
        path: String,
        args: serde_json::Value,
        cascade: serde_json::Map<String, serde_json::Value>,
        outcome: InvokeOutcome,
    },
}

/// How a recorded invoke ended. `Failure` carries the identity hash of the
/// child's failure (kind + message) — recorded even when the caller catches
/// the thrown error, so a parent memoized with a fallback output revalidates
/// only while the child still fails identically, and rebuilds when the child
/// is fixed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum InvokeOutcome {
    Output(Hash),
    Failure(Hash),
}

/// Console line severity: the levels the `console` shim emits (`info` maps
/// to `log`; unknown strings read as `Log` at the op boundary). Part of the
/// memoized log output, hence store-side; the CLI wire keeps the lowercase
/// strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Log,
    Debug,
    Warn,
    Error,
}

impl LogLevel {
    pub fn parse(s: &str) -> LogLevel {
        match s {
            "debug" => LogLevel::Debug,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Log,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Log => "log",
            LogLevel::Debug => "debug",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One captured console line from a build. Lives here (not odm-js) so memo
/// entries can replay logs on a hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    pub level: LogLevel,
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
