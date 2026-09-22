//! The JS-executor seam: everything the scheduler needs from a JS host,
//! as a trait plus the value types that cross it. The native executor
//! (odm-js: V8 isolates from a snapshot) implements [`Executor`]; the web
//! export's runtime implements it against the page's own JS. odm-build
//! itself stays V8-free, so the build engine compiles for wasm.

use crate::version::ApiVersion;
use odm_ir::Hash;
use odm_store::Dep;
pub use odm_store::{LogLevel, LogLine};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Sentinel hashed for a cascade key that is absent from the environment.
pub(crate) const MISSING_CASCADE: &[u8] = b"__odm_missing__";

/// Canonical hash of a cascade value as recorded in `Dep::Cascade`
/// (missing keys hash to a distinct sentinel). Scheduler validation and the
/// executor's dep recording must both use exactly this function.
pub fn cascade_value_hash(v: Option<&Value>) -> Hash {
    match v {
        Some(v) => odm_ir::hash_json(v),
        None => Hash::of_bytes(MISSING_CASCADE),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// Syntax or runtime error in part JS; agent-readable, includes stack.
    #[error("{0}")]
    Js(String),
    #[error("build cancelled")]
    Cancelled,
    #[error("build output invalid: {0}")]
    BadOutput(String),
    #[error("internal: {0}")]
    Internal(String),
}

/// A failed build plus the console output it produced before failing. Logs
/// stay data all the way up — each surface (CLI JSON, viewer panel) decides
/// how to show them next to the error.
#[derive(Debug)]
pub struct FailedBuild {
    pub error: BuildError,
    pub logs: Vec<LogLine>,
    /// Deps recorded up to the failure, so the scheduler can memoize pure
    /// failures with entries that validate like any other.
    pub deps: Vec<Dep>,
}

impl std::fmt::Display for FailedBuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl From<BuildError> for FailedBuild {
    fn from(error: BuildError) -> FailedBuild {
        FailedBuild { error, logs: Vec::new(), deps: Vec::new() }
    }
}

/// A failed nested invoke, as the executor's invoke op records and rethrows.
pub struct InvokeError {
    /// Agent-readable message; becomes the thrown JS error's text.
    pub message: String,
    /// Identity hash of the child's failure value ((kind, message), computed
    /// by the scheduler) — recorded as a failed-invoke dep so a parent that
    /// catches the throw still depends on the broken child. `None` when the
    /// failure is not a value (cancellation): nothing is recorded.
    pub identity: Option<Hash>,
}

impl From<String> for InvokeError {
    fn from(message: String) -> InvokeError {
        InvokeError { message, identity: None }
    }
}

/// Nested-build callback, provided by the scheduler. Runs on the calling
/// worker thread; the executor may create nested (LIFO) build contexts under
/// it. Returns the hash of the invoked part's output Node in the store.
pub trait Invoker {
    fn invoke(
        &mut self,
        path: &str,
        args: &Value,
        cascade: &serde_json::Map<String, Value>,
    ) -> Result<Hash, InvokeError>;
}

/// Handle to interrupt a running build from another thread (native: V8
/// `TerminateExecution`). Safe to call repeatedly — a lone interrupt can be
/// swallowed, so cancellation re-fires until the build returns.
pub trait BuildInterrupt: Send + Sync {
    fn interrupt(&self);
}

pub type InterruptHandle = Arc<dyn BuildInterrupt>;

pub struct BuildInput<'a> {
    /// Project-relative path, used for module identity + error messages.
    pub path: &'a str,
    pub code: &'a str,
    /// API version from the file's `//! ODM API <version>` pragma; selects the
    /// framework surface this build runs against.
    pub api: ApiVersion,
    /// Effective args: caller args validated against the file's declared
    /// inputs, with defaults merged in by the scheduler.
    pub args: &'a Value,
    /// Input declarations for `ctx.input` routing/hydration, as JSON:
    /// `{ name: { cascade: bool, type: string|null } }`.
    pub decls: &'a Value,
    /// The build's environment: cascade input values by name.
    pub cascade: &'a HashMap<String, Value>,
    pub kernel: Arc<odm_kernel::Kernel>,
    pub store: Arc<odm_store::Store>,
    pub cancel: Option<odm_kernel::CancelToken>,
    /// Callback for nested `ctx.invoke()`; None makes invoke fail.
    pub invoker: Option<Box<dyn Invoker>>,
    /// Called with the interrupt handle before any part code runs
    /// (module top level included); the scheduler may use it to interrupt
    /// from another thread. Executors without cross-thread interruption
    /// (the web runtime) may never call it.
    pub on_handle: Option<Box<dyn FnOnce(InterruptHandle)>>,
}

#[derive(Debug)]
pub struct BuildOutput {
    /// Hash of the output `Node` in the store.
    pub output: Hash,
    pub deps: Vec<Dep>,
    pub logs: Vec<LogLine>,
}

/// Default watchdog for [`Executor::extract_export`] module evaluation. Top
/// level should be milliseconds; anything near this makes every build of the
/// file unusably slow anyway.
pub const EXTRACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A JS host: runs part builds and reads module exports. One per
/// process/page; must behave identically across hosts — every published
/// result stays byte-equivalent to a from-scratch build regardless of which
/// executor ran it.
pub trait Executor: Send + Sync {
    /// Run one part build in a fresh module scope.
    fn run_build(&self, input: BuildInput<'_>) -> Result<BuildOutput, FailedBuild>;

    /// Load a part module (without calling its build()) and return one
    /// of its exports as JSON — `None` if the export is absent. The module's
    /// top level runs, so it gets real ops. Callers have no cancel plumbing,
    /// so a watchdog bounds top-level evaluation by `timeout`.
    #[allow(clippy::too_many_arguments)]
    fn extract_export(
        &self,
        path: &str,
        code: &str,
        api: ApiVersion,
        export: &str,
        kernel: Arc<odm_kernel::Kernel>,
        store: Arc<odm_store::Store>,
        timeout: std::time::Duration,
    ) -> Result<Option<Value>, BuildError>;
}
