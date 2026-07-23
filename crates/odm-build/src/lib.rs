//! Build system: generations, demand-driven memoized builds, in-flight
//! dedup across threads, cycle detection, cancellation.
//!
//! Model: a *pass* = (generation, context). Builds are demand-driven from the
//! root doohickey; nested `invoke()`s run inline on the requesting thread in
//! their own disposable isolates (LIFO nesting). Memo lookup is by
//! (code hash, args hash) with Salsa-style validation of recorded deps.
//! Consistency invariant: every published result is byte-equivalent to a
//! from-scratch build of its generation.

mod registry;
mod scheduler;
mod sources;

pub use scheduler::{BuildEngine, BuildFailure, FailureKind, Pass, PassResult, Stats, SyncResult};
pub use sources::{Animation, Manifest, ProjectSnapshot, ScanError, scan_project};
