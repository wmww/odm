//! Build system: generations, demand-driven memoized builds, in-flight
//! dedup across threads, cycle detection, cancellation.
//!
//! Model: a *pass* = (generation, context). Builds are demand-driven from the
//! root doohickey; nested `invoke()`s run inline on the requesting thread in
//! their own disposable isolates (LIFO nesting). Memo lookup is by
//! (code hash, args hash) with Salsa-style validation of recorded deps.
//! Consistency invariant: every published result is byte-equivalent to a
//! from-scratch build of its generation.

pub mod meta;
mod registry;
mod report;
mod scheduler;
mod sources;

pub use meta::{ExtType, Input, Meta};
pub use report::{InputReport, ReportEntry, check_set_names};
pub use scheduler::{
    BuildEngine, BuildFailure, DEFAULT_ROOT, FailureKind, Pass, PassResult, Stats, SyncResult,
    View,
};
pub use sources::{
    ENGINE_VERSION, ProjectMarker, ProjectSnapshot, ScanError, Source, is_project, read_marker,
    scan_project, sync_marker,
};
