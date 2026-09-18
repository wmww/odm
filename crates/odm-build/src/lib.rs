//! Build system: generations, demand-driven memoized builds, in-flight
//! dedup across threads, cycle detection, cancellation.
//!
//! Model: a *pass* = (generation, view). Builds are demand-driven from the
//! root doohickey; nested `invoke()`s run inline on the requesting thread in
//! their own disposable isolates (LIFO nesting). Memo lookup is by
//! (code hash, args hash) with Salsa-style validation of recorded deps.
//! Consistency invariant: every published result is byte-equivalent to a
//! from-scratch build of its generation.

pub mod executor;
mod ir_json;
pub mod meta;
mod registry;
mod report;
mod scheduler;
mod sources;
mod version;

pub use executor::{
    BuildError, BuildInput, BuildInterrupt, BuildOutput, EXTRACT_TIMEOUT, Executor, FailedBuild,
    InterruptHandle, InvokeError, Invoker, LogLevel, LogLine, cascade_value_hash,
};
pub use ir_json::node_from_json;
pub use meta::{ExtType, Input, Meta, synthesize, synthesize_variant, tag_name};
pub use version::{ApiVersion, SUPPORTED, parse_doc, parse_pragma};
pub use report::{
    InputKind, InputReport, ReportEntry, ValueSource, check_input_names, declared_entries,
};
pub use scheduler::{
    BuildEngine, BuildFailure, BuildStats, DEFAULT_ROOT, FailureKind, Pass, PassResult, Stats,
    SyncResult, View,
};
pub use sources::{
    ENGINE_VERSION, EXPORT_MARKER, ProjectMarker, ProjectSnapshot, ScanError, Source, Units,
    create_project, is_project, read_marker, scan_project, sync_marker, sync_marker_as,
};
