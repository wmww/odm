//! JS runtime: runs doohickey `build()` functions in disposable V8 isolates
//! created from a snapshot with the ODM framework + three.js subset preloaded.
//!
//! Thread rules (see notes/spike-findings.md): isolates nest
//! strictly LIFO on a thread — a nested invoke creates its own isolate, uses
//! it, and drops it before the outer isolate resumes. Never interleave.

mod ir_json;
mod ops;
mod session;
mod snapshot;
mod version;

pub use ir_json::node_from_json;
pub use session::{Invoker, LogLine, SessionState};
pub use snapshot::JsEnv;
pub use version::{ApiVersion, SUPPORTED, parse_doc, parse_pragma};

/// Re-export so downstream crates can hold isolate handles without a direct
/// deno_core dependency.
pub use deno_core::v8::IsolateHandle;

/// Canonical hash of a cascade value as recorded in `Dep::Cascade`
/// (missing keys hash to a distinct sentinel). The scheduler must use this
/// exact function when validating memo entries.
pub fn cascade_value_hash(v: Option<&Value>) -> Hash {
    match v {
        Some(v) => odm_ir::hash_json(v),
        None => Hash::of_bytes(ops::MISSING_CASCADE),
    }
}

use deno_core::error::JsError;
use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions, serde_v8, v8};
use odm_ir::Hash;
use odm_store::Dep;
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// Syntax or runtime error in doohickey JS; agent-readable, includes stack.
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
}

impl std::fmt::Display for FailedBuild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl From<BuildError> for FailedBuild {
    fn from(error: BuildError) -> FailedBuild {
        FailedBuild { error, logs: Vec::new() }
    }
}

pub struct BuildInput<'a> {
    /// Project-relative path, used for module specifier + error messages.
    pub path: &'a str,
    pub code: &'a str,
    /// API version from the file's `//! odm <version>` pragma; picks the
    /// framework snapshot this build's isolate is created from.
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
    /// Called with the isolate handle once JS is about to run build(); the
    /// scheduler may use it to TerminateExecution from another thread.
    /// (Deliberately not called during module evaluation — rusty_v8 #830.)
    pub on_isolate: Option<Box<dyn FnOnce(v8::IsolateHandle)>>,
}

#[derive(Debug)]
pub struct BuildOutput {
    /// Hash of the output `Node` in the store.
    pub output: Hash,
    pub deps: Vec<Dep>,
    pub logs: Vec<LogLine>,
}

/// Run one doohickey build in a fresh disposable isolate.
pub fn run_build(env: &JsEnv, input: BuildInput<'_>) -> Result<BuildOutput, FailedBuild> {
    let specifier = snapshot::doohickey_specifier(input.path)
        .map_err(|e| BuildError::Internal(format!("bad doohickey path {:?}: {e}", input.path)))?;

    let loader = snapshot::DoohickeyLoader::new(input.api, specifier.clone(), input.code.to_string());
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

    let BuildInput { path, args, decls, cascade, kernel, store, cancel, invoker, on_isolate, .. } =
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

    let take_session = |rt: &mut JsRuntime| -> SessionState {
        rt.op_state().borrow_mut().take::<SessionState>()
    };

    // Load + evaluate the doohickey module (sync loader; the event loop
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

    if let Some(cb) = on_isolate {
        cb(rt.v8_isolate().thread_safe_handle());
    }

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

    let output = match ir_json::node_from_json(&store, &ir_value) {
        Ok(o) => o,
        Err(m) => return Err(failed(BuildError::BadOutput(m), &mut session)),
    };
    Ok(BuildOutput {
        output,
        deps: std::mem::take(&mut session.deps),
        logs: std::mem::take(&mut session.logs),
    })
}

/// Load a doohickey module (without calling its build()) and return one of
/// its exports as JSON — `None` if the export is absent. The conformance
/// runner reads `export const checks` this way. The module's top level runs,
/// so it gets a real session (ops work) under the version its pragma picks.
pub fn extract_export(
    env: &JsEnv,
    path: &str,
    code: &str,
    api: ApiVersion,
    export: &str,
    kernel: Arc<odm_kernel::Kernel>,
    store: Arc<odm_store::Store>,
) -> Result<Option<Value>, BuildError> {
    let specifier = snapshot::doohickey_specifier(path)
        .map_err(|e| BuildError::Internal(format!("bad doohickey path {path:?}: {e}")))?;
    let loader = snapshot::DoohickeyLoader::new(api, specifier.clone(), code.to_string());
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

    let mod_id = futures::executor::block_on(async {
        let id = rt.load_main_es_module(&specifier).await?;
        let eval = rt.mod_evaluate(id);
        rt.run_event_loop(PollEventLoopOptions::default()).await?;
        eval.await.map(|_| id)
    })
    .map_err(|e| BuildError::Js(format!("{path}: {e}")))?;

    let ns_global = rt
        .get_module_namespace(mod_id)
        .map_err(|e| BuildError::Internal(format!("module namespace: {e}")))?;
    deno_core::scope!(scope, &mut rt);
    let ns = v8::Local::new(scope, ns_global);
    let key = v8::String::new(scope, export)
        .ok_or_else(|| BuildError::Internal("export name to v8".into()))?;
    let Some(value) = ns.get(scope, key.into()) else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    serde_v8::from_v8::<Value>(scope, value)
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
    FailedBuild { error, logs: std::mem::take(&mut session.logs) }
}
