// One integration-test binary instead of eight. Every test binary links the
// full V8/engine stack, so each extra file in tests/ costs a ~quarter-GB link.
//
// One JsEnv for the whole suite: V8 seeds a process-wide read-only heap from
// the first snapshot, and creating one while another thread executes JS aborts
// the process (notes/spike-findings.md, "Snapshot count/concurrency"). With a
// per-module env, two modules' tests racing was exactly that abort.
use odm_js::JsEnv;
use std::sync::{Arc, OnceLock};

pub fn env() -> Arc<JsEnv> {
    static ENV: OnceLock<Arc<JsEnv>> = OnceLock::new();
    ENV.get_or_init(|| Arc::new(JsEnv::new().unwrap())).clone()
}

mod build;
mod doctests;
mod examples;
mod marker;
mod meta_extract;
mod new_project;
mod report;
mod versions;
