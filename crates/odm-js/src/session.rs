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
    /// The build's environment: cascade input values by name (values
    /// provided along the invoke chain plus declaration defaults),
    /// resolved by the scheduler for this exact invoke path.
    pub cascade: HashMap<String, Value>,
    pub cancel: Option<CancelToken>,
    pub deps: Vec<Dep>,
    pub logs: Vec<LogLine>,
    pub invoker: Option<Box<dyn Invoker>>,
}

/// Console line; defined in odm-store so memo entries can carry logs.
pub use odm_store::{LogLevel, LogLine};

/// A failed nested invoke, as `op_invoke` records and rethrows it.
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
/// worker thread; may create its own (LIFO-nested) isolate. Returns the hash
/// of the invoked doohickey's output Node in the store.
pub trait Invoker {
    fn invoke(
        &mut self,
        path: &str,
        args: &Value,
        cascade: &serde_json::Map<String, Value>,
    ) -> Result<Hash, InvokeError>;
}
