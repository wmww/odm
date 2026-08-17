use odm_build::Invoker;
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
