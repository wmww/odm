use odm_ir::Hash;
use odm_kernel::{CancelToken, Kernel};
use odm_store::{Dep, Store};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Per-build state living in the isolate's OpState.
pub struct SessionState {
    pub kernel: Arc<Kernel>,
    pub store: Arc<Store>,
    /// Context values by full key: "t", "params.<name>", ...
    pub context: HashMap<String, Value>,
    pub cancel: Option<CancelToken>,
    pub deps: Vec<Dep>,
    pub logs: Vec<LogLine>,
    pub invoker: Option<Box<dyn Invoker>>,
}

/// Console line; defined in odm-store so memo entries can carry logs.
pub use odm_store::LogLine;

pub struct InvokeResult {
    /// Hash of the invoked doohickey's output Node in the store.
    pub output: Hash,
    /// The output as IR JSON (embedded into the caller's tree).
    pub tree: Value,
}

/// Nested-build callback, provided by the scheduler. Runs on the calling
/// worker thread; may create its own (LIFO-nested) isolate.
pub trait Invoker {
    fn invoke(&mut self, path: &str, args: &Value) -> Result<InvokeResult, String>;
}
