//! The V8 rules the snapshot design is built on (measured 2026-07-29,
//! deno_core 0.408 / its pinned V8):
//!
//! - Sequential snapshot creation in one process: fine, IF the blobs are
//!   structurally identical (same modules/external refs — e.g. two JsEnvs).
//! - Structurally DIFFERENT blobs cannot coexist: V8 shares one read-only
//!   heap per process, seeded by the first blob used; deserializing another
//!   shape dies on external-reference indexes ("Check failed:
//!   index < size()"). This is why API versions share ONE snapshot with
//!   per-isolate surface selection instead of a snapshot per version.
//! - Creation nested under a suspended isolate on the SAME thread: fine.
//! - Creation while ANY other thread executes JS: process abort
//!   ("Check failed: IsFreeSpaceOrFiller" or a libc++ vector bounds abort —
//!   the shared read-only heap is mutated during creation).
//!
//! Hence `JsEnv::new` builds the one snapshot before any isolate exists,
//! once per process. The two `#[ignore]`d tests reproduce the aborts; run
//! one by name (never both, never with others) to re-check a V8 upgrade.

use odm_js::{BuildInput, JsEnv, run_build};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// The tests here create snapshots mid-test, so two of them running on
/// parallel test threads reproduce exactly the abort described above.
/// Every test takes this lock for its whole body.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn build_box(env: &JsEnv) -> Result<(), String> {
    let store = odm_store::Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let out = run_build(
        env,
        BuildInput {
            path: "main.js",
            code: "export default function build() { return odm.box(1); }",
            api: odm_js::ApiVersion::Unstable,
            args: &json!({}),
            decls: &json!({}),
            cascade: &HashMap::<String, Value>::new(),
            kernel,
            store,
            cancel: None,
            invoker: None,
            on_isolate: None,
        },
    )
    .map_err(|e| e.to_string())?;
    assert!(!out.output.to_string().is_empty());
    Ok(())
}

#[test]
fn two_snapshots_one_process() {
    let _lock = exclusive();
    let env1 = JsEnv::new().expect("first snapshot");
    build_box(&env1).expect("build from first snapshot");
    let env2 = JsEnv::new().expect("second snapshot");
    build_box(&env2).expect("build from second snapshot");
    // Interleave: the first env must still work after the second exists.
    build_box(&env1).expect("first snapshot again");
}

#[test]
#[ignore = "aborts the whole process by design (SIGSEGV in V8); run alone to re-verify"]
fn concurrent_snapshot_creation_aborts() {
    let _lock = exclusive();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            std::thread::spawn(|| {
                let env = JsEnv::new().expect("snapshot on thread");
                build_box(&env).expect("build on thread");
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
}

/// The cross-version nested-invoke scenario for a lazy snapshot registry:
/// while one isolate is suspended inside op_invoke, create a second
/// snapshot on the same thread and build from it.
#[test]
fn snapshot_creation_inside_invoke() {
    let _lock = exclusive();
    struct SnapshottingInvoker;
    impl odm_js::Invoker for SnapshottingInvoker {
        fn invoke(
            &mut self,
            _path: &str,
            _args: &Value,
            _cascade: &serde_json::Map<String, Value>,
        ) -> Result<odm_ir::Hash, String> {
            let env2 = JsEnv::new().map_err(|e| format!("nested snapshot: {e}"))?;
            let store = odm_store::Store::new();
            let kernel = odm_kernel::Kernel::new(store.clone());
            let out = run_build(
                &env2,
                BuildInput {
                    path: "inner.js",
                    code: "export default function build() { return odm.box(1); }",
                    api: odm_js::ApiVersion::Unstable,
                    args: &json!({}),
                    decls: &json!({}),
                    cascade: &HashMap::<String, Value>::new(),
                    kernel,
                    store,
                    cancel: None,
                    invoker: None,
                    on_isolate: None,
                },
            )
            .map_err(|e| e.to_string())?;
            Ok(out.output)
        }
    }

    let env1 = JsEnv::new().expect("first snapshot");
    let store = odm_store::Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    // The inner output hash is not interned in this store, so return a box
    // from the outer build and ignore the Instance.
    let out = run_build(
        &env1,
        BuildInput {
            path: "outer.js",
            code: "export default function build(ctx) { ctx.invoke('inner.js'); return odm.box(2); }",
            api: odm_js::ApiVersion::Unstable,
            args: &json!({}),
            decls: &json!({}),
            cascade: &HashMap::<String, Value>::new(),
            kernel,
            store,
            cancel: None,
            invoker: Some(Box::new(SnapshottingInvoker)),
            on_isolate: None,
        },
    );
    out.expect("outer build with nested snapshot creation");
}

#[test]
#[ignore = "aborts the whole process by design (SIGABRT in V8); run alone to re-verify"]
fn snapshot_creation_during_builds_aborts() {
    let _lock = exclusive();
    let env1 = JsEnv::new().expect("first snapshot");
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        let builder = s.spawn(|| {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                build_box(&env1).expect("build during snapshot creation");
            }
        });
        let env2 = JsEnv::new().expect("snapshot while builds run");
        build_box(&env2).expect("build from new snapshot");
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        builder.join().expect("builder thread panicked");
    });
}
