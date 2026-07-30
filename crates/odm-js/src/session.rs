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
    /// The build's environment: cascade input values by name (explicit
    /// provides from the invoke chain plus auto-provided declaration
    /// defaults), resolved by the scheduler for this exact invoke path.
    pub context: HashMap<String, Value>,
    pub cancel: Option<CancelToken>,
    pub deps: Vec<Dep>,
    pub logs: Vec<LogLine>,
    pub invoker: Option<Box<dyn Invoker>>,
}

/// Console line; defined in odm-store so memo entries can carry logs.
pub use odm_store::LogLine;

/// Nested-build callback, provided by the scheduler. Runs on the calling
/// worker thread; may create its own (LIFO-nested) isolate. Returns the hash
/// of the invoked doohickey's output Node in the store.
pub trait Invoker {
    fn invoke(
        &mut self,
        path: &str,
        args: &Value,
        provides: &serde_json::Map<String, Value>,
    ) -> Result<Hash, String>;
}
