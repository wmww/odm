//! JS runtime: runs part `build()` functions in disposable V8 isolates
//! created from a snapshot with the ODM framework + three.js subset preloaded.
//!
//! Thread rules (see notes/spike-findings.md): isolates nest
//! strictly LIFO on a thread — a nested invoke creates its own isolate, uses
//! it, and drops it before the outer isolate resumes. Never interleave.

mod ops;
mod session;
mod snapshot;

pub use session::SessionState;
pub use snapshot::JsEnv;
/// The per-version surface tables. Public so the web export's mirror of
/// them (odm-export `bundle.rs`) can be drift-tested against the originals.
pub use snapshot::{PART_IMPORT_ERROR, version_manifest};

// The executor seam's types live in odm-build; re-export the ones this
// crate's callers use alongside the V8 implementation.
pub use odm_build::{
    ApiVersion, BuildError, BuildInput, BuildOutput, EXTRACT_TIMEOUT, FailedBuild,
    InterruptHandle, InvokeError, Invoker, LogLevel, LogLine, node_from_json,
};

use deno_core::error::JsError;
use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions, serde_v8, v8};
use odm_build::BuildInterrupt;
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// [`InterruptHandle`] over V8 `TerminateExecution`. Terminating during
/// module evaluation is safe: deno_core's mod_evaluate handles it, and the
/// rusty_v8 #830 / v8 12379 crash needs top-level await machinery
/// (stress-tested clean on v8 149 — see runtime.rs termination tests).
struct IsolateInterrupt(v8::IsolateHandle);

impl BuildInterrupt for IsolateInterrupt {
    fn interrupt(&self) {
        self.0.terminate_execution();
    }
}

/// The native executor: one fresh disposable isolate per build, created
/// from the framework snapshot.
impl odm_build::Executor for JsEnv {
    fn run_build(&self, input: BuildInput<'_>) -> Result<BuildOutput, FailedBuild> {
        run_build(self, input)
    }

    fn extract_export(
        &self,
        path: &str,
        code: &str,
        api: ApiVersion,
        export: &str,
        kernel: Arc<odm_kernel::Kernel>,
        store: Arc<odm_store::Store>,
        timeout: std::time::Duration,
    ) -> Result<Option<Value>, BuildError> {
        extract_export(self, path, code, api, export, kernel, store, timeout)
    }
}

/// Run one part build in a fresh disposable isolate.
pub fn run_build(env: &JsEnv, input: BuildInput<'_>) -> Result<BuildOutput, FailedBuild> {
    let specifier = snapshot::part_specifier(input.path)
        .map_err(|e| BuildError::Internal(format!("bad part path {:?}: {e}", input.path)))?;

    let loader = snapshot::PartLoader::new(specifier.clone(), input.code.to_string());
    let mut rt = JsRuntime::new(RuntimeOptions {
        startup_snapshot: Some(env.snapshot()),
        module_loader: Some(Rc::new(loader)),
        extensions: vec![ops::odm_ops::init()],
        ..Default::default()
    });

    // Install the API surface the file's pragma selects, before its module
    // (whose top level may already use `odm`/`THREE`) loads.
    rt.execute_script("odm:select-version", snapshot::select_version_script(input.api))
        .map_err(|e| BuildError::Internal(format!("select api version {}: {e}", input.api)))?;

    let BuildInput { path, args, decls, cascade, kernel, store, cancel, invoker, on_handle, .. } =
        input;
    let session = SessionState {
        kernel,
        store: store.clone(),
        cascade: cascade.clone(),
        cancel: cancel.clone(),
        deps: Vec::new(),
        logs: Vec::new(),
        invoker,
    };
    rt.op_state().borrow_mut().put(session);

    // Hand out the interrupt handle before the module (whose top level is
    // arbitrary part code) evaluates, so even `while(true){}` outside
    // build() stays terminable.
    if let Some(cb) = on_handle {
        let handle: InterruptHandle =
            Arc::new(IsolateInterrupt(rt.v8_isolate().thread_safe_handle()));
        cb(handle);
    }

    let take_session = |rt: &mut JsRuntime| -> SessionState {
        rt.op_state().borrow_mut().take::<SessionState>()
    };

    // Load + evaluate the part module (sync loader; the event loop
    // settles immediately).
    let mod_id = {
        let load = futures::executor::block_on(rt.load_main_es_module(&specifier));
        match load {
            Ok(id) => {
                let eval = rt.mod_evaluate(id);
                let run = futures::executor::block_on(async {
                    rt.run_event_loop(PollEventLoopOptions::default()).await?;
                    eval.await
                });
                if let Err(e) = run {
                    return Err(js_error(path, take_session(&mut rt), e.to_string()));
                }
                id
            }
            Err(e) => {
                return Err(js_error(path, take_session(&mut rt), e.to_string()));
            }
        }
    };

    // Call __odm.runBuild(namespace, args) and pull the IR JSON back.
    let ns_global = rt
        .get_module_namespace(mod_id)
        .map_err(|e| BuildError::Internal(format!("module namespace: {e}")))?;

    let ir_value: Result<Value, BuildError> = {
        deno_core::scope!(scope, &mut rt);
        let context = scope.get_current_context();
        let global = context.global(scope);

        let run_build_fn: v8::Local<v8::Function> = (|| {
            let odm_key = v8::String::new(scope, "__odm")?;
            let odm = global.get(scope, odm_key.into())?.to_object(scope)?;
            let key = v8::String::new(scope, "runBuild")?;
            let f = odm.get(scope, key.into())?;
            v8::Local::<v8::Function>::try_from(f).ok()
        })()
        .ok_or_else(|| BuildError::Internal("__odm.runBuild missing from snapshot".into()))?;

        let ns = v8::Local::new(scope, ns_global);
        let args_v8 = serde_v8::to_v8(scope, args)
            .map_err(|e| BuildError::Internal(format!("args to v8: {e}")))?;
        let decls_v8 = serde_v8::to_v8(scope, decls)
            .map_err(|e| BuildError::Internal(format!("decls to v8: {e}")))?;

        v8::tc_scope!(let tc, scope);
        let recv = v8::undefined(tc);
        let ret = run_build_fn.call(tc, recv.into(), &[ns.into(), args_v8, decls_v8]);
        match ret {
            Some(v) => serde_v8::from_v8::<Value>(tc, v)
                .map_err(|e| BuildError::BadOutput(format!("build() output not serializable: {e}"))),
            None => {
                if tc.is_execution_terminating() || tc.has_terminated() {
                    Err(BuildError::Cancelled)
                } else if let Some(exn) = tc.exception() {
                    let js_err = JsError::from_v8_exception(tc, exn);
                    Err(BuildError::Js(format!("{}: {}", path, format_js_error(&js_err))))
                } else {
                    Err(BuildError::Internal("build() call failed with no exception".into()))
                }
            }
        }
    };

    let mut session = take_session(&mut rt);
    let failed = |error: BuildError, session: &mut SessionState| FailedBuild {
        error,
        logs: std::mem::take(&mut session.logs),
        deps: std::mem::take(&mut session.deps),
    };
    // Cancelled kernel ops surface as JS exceptions; prefer the Cancelled signal.
    if let Some(tok) = &session.cancel
        && tok.is_cancelled()
    {
        return Err(failed(BuildError::Cancelled, &mut session));
    }
    let ir_value = match ir_value {
        Ok(v) => v,
        Err(e) => return Err(failed(e, &mut session)),
    };

    let output = match node_from_json(&store, &ir_value) {
        Ok(o) => o,
        Err(m) => return Err(failed(BuildError::BadOutput(m), &mut session)),
    };
    Ok(BuildOutput {
        output,
        deps: std::mem::take(&mut session.deps),
        logs: std::mem::take(&mut session.logs),
    })
}

/// Load a part module (without calling its build()) and return one of
/// its exports as JSON — `None` if the export is absent. The conformance
/// runner reads `export const checks` this way. The module's top level runs,
/// so it gets a real session (ops work) under the version its pragma picks.
/// Callers have no cancel plumbing, so a watchdog terminates top-level
/// evaluation after `timeout` (else `while(true){}` would wedge the caller).
pub fn extract_export(
    env: &JsEnv,
    path: &str,
    code: &str,
    api: ApiVersion,
    export: &str,
    kernel: Arc<odm_kernel::Kernel>,
    store: Arc<odm_store::Store>,
    timeout: std::time::Duration,
) -> Result<Option<Value>, BuildError> {
    let specifier = snapshot::part_specifier(path)
        .map_err(|e| BuildError::Internal(format!("bad part path {path:?}: {e}")))?;
    let loader = snapshot::PartLoader::new(specifier.clone(), code.to_string());
    let mut rt = JsRuntime::new(RuntimeOptions {
        startup_snapshot: Some(env.snapshot()),
        module_loader: Some(Rc::new(loader)),
        extensions: vec![ops::odm_ops::init()],
        ..Default::default()
    });
    rt.execute_script("odm:select-version", snapshot::select_version_script(api))
        .map_err(|e| BuildError::Internal(format!("select api version {api}: {e}")))?;
    rt.op_state().borrow_mut().put(SessionState {
        kernel,
        store,
        cascade: HashMap::new(),
        cancel: None,
        deps: Vec::new(),
        logs: Vec::new(),
        invoker: None,
    });

    let handle = rt.v8_isolate().thread_safe_handle();
    let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let timed_out2 = timed_out.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        let mut wait = timeout;
        // A lone terminate can be swallowed (see Pass::cancel in odm-build);
        // keep re-terminating until evaluation actually returns.
        while done_rx.recv_timeout(wait) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
            timed_out2.store(true, std::sync::atomic::Ordering::SeqCst);
            handle.terminate_execution();
            wait = std::time::Duration::from_millis(100);
        }
    });

    let mod_id = futures::executor::block_on(async {
        let id = rt.load_main_es_module(&specifier).await?;
        let eval = rt.mod_evaluate(id);
        rt.run_event_loop(PollEventLoopOptions::default()).await?;
        eval.await.map(|_| id)
    });
    let _ = done_tx.send(());
    let _ = watchdog.join();
    let timed_out = timed_out.load(std::sync::atomic::Ordering::SeqCst);
    if timed_out {
        // Terminate flag may still be pending if eval finished in the race
        // window; clear it so the namespace reads below aren't poisoned.
        rt.v8_isolate().cancel_terminate_execution();
    }
    let mod_id = mod_id.map_err(|e| {
        if timed_out {
            BuildError::Js(format!(
                "{path}: module evaluation timed out after {timeout:?} \
                 (infinite loop or stalled await at top level?)"
            ))
        } else {
            BuildError::Js(format!("{path}: {e}"))
        }
    })?;

    let ns_global = rt
        .get_module_namespace(mod_id)
        .map_err(|e| BuildError::Internal(format!("module namespace: {e}")))?;
    deno_core::scope!(scope, &mut rt);
    let ns = v8::Local::new(scope, ns_global);
    let key = v8::String::new(scope, export)
        .ok_or_else(|| BuildError::Internal("export name to v8".into()))?;

    // Serialize in JS (`__odm.exportJson`), through the same replacer as
    // invoke args: THREE instances become their wire form before the
    // boundary, and the engine never sees another spelling.
    let global = scope.get_current_context().global(scope);
    let export_fn: v8::Local<v8::Function> = (|| {
        let odm_key = v8::String::new(scope, "__odm")?;
        let odm = global.get(scope, odm_key.into())?.to_object(scope)?;
        let key = v8::String::new(scope, "exportJson")?;
        let f = odm.get(scope, key.into())?;
        v8::Local::<v8::Function>::try_from(f).ok()
    })()
    .ok_or_else(|| BuildError::Internal("__odm.exportJson missing from snapshot".into()))?;
    v8::tc_scope!(let tc, scope);
    let recv = v8::undefined(tc);
    let Some(value) = export_fn.call(tc, recv.into(), &[ns.into(), key.into()]) else {
        let message = match tc.exception() {
            Some(exn) => format_js_error(&JsError::from_v8_exception(tc, exn)),
            None => "unknown error".into(),
        };
        return Err(BuildError::BadOutput(format!("export {export:?} not serializable: {message}")));
    };
    if value.is_undefined() {
        return Ok(None);
    }
    let text = value.to_rust_string_lossy(tc);
    serde_json::from_str::<Value>(&text)
        .map(Some)
        .map_err(|e| BuildError::BadOutput(format!("export {export:?} not serializable: {e}")))
}

fn format_js_error(e: &JsError) -> String {
    let mut out = e.exception_message.clone();
    for frame in e.frames.iter().take(8) {
        let loc = match (&frame.file_name, frame.line_number) {
            (Some(f), Some(l)) => format!("{f}:{l}"),
            (Some(f), None) => f.clone(),
            _ => "<anon>".into(),
        };
        let func = frame.function_name.as_deref().unwrap_or("<anonymous>");
        out.push_str(&format!("\n    at {func} ({loc})"));
    }
    out
}

fn js_error(path: &str, mut session: SessionState, msg: String) -> FailedBuild {
    let error = if session.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        BuildError::Cancelled
    } else {
        BuildError::Js(format!("{path}: {msg}"))
    };
    FailedBuild {
        error,
        logs: std::mem::take(&mut session.logs),
        deps: std::mem::take(&mut session.deps),
    }
}
