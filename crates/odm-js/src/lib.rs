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

pub use ir_json::node_from_json;
pub use session::{Invoker, LogLine, SessionState};
pub use snapshot::JsEnv;

/// Re-export so downstream crates can hold isolate handles without a direct
/// deno_core dependency.
pub use deno_core::v8::IsolateHandle;

/// Canonical hash of a context value as recorded in `Dep::Context`
/// (missing keys hash to a distinct sentinel). The scheduler must use this
/// exact function when validating memo entries.
pub fn context_value_hash(v: Option<&Value>) -> Hash {
    match v {
        Some(v) => odm_ir::hash_json(v),
        None => Hash::of_bytes(ops::MISSING_CONTEXT),
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

pub struct BuildInput<'a> {
    /// Project-relative path, used for module specifier + error messages.
    pub path: &'a str,
    pub code: &'a str,
    pub args: &'a Value,
    /// Full context map: `t`, `params.<name>`, ...
    pub context: &'a HashMap<String, Value>,
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
pub fn run_build(env: &JsEnv, input: BuildInput<'_>) -> Result<BuildOutput, BuildError> {
    let specifier = snapshot::doohickey_specifier(input.path)
        .map_err(|e| BuildError::Internal(format!("bad doohickey path {:?}: {e}", input.path)))?;

    let loader = snapshot::DoohickeyLoader::new(specifier.clone(), input.code.to_string());
    let mut rt = JsRuntime::new(RuntimeOptions {
        startup_snapshot: Some(env.snapshot()),
        module_loader: Some(Rc::new(loader)),
        extensions: vec![ops::odm_ops::init()],
        ..Default::default()
    });

    let BuildInput { path, args, context, kernel, store, cancel, invoker, on_isolate, .. } = input;
    let session = SessionState {
        kernel,
        store: store.clone(),
        context: context.clone(),
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

        v8::tc_scope!(let tc, scope);
        let recv = v8::undefined(tc);
        let ret = run_build_fn.call(tc, recv.into(), &[ns.into(), args_v8]);
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
    // Cancelled kernel ops surface as JS exceptions; prefer the Cancelled signal.
    if let Some(tok) = &session.cancel
        && tok.is_cancelled()
    {
        return Err(BuildError::Cancelled);
    }
    let ir_value = match ir_value {
        Ok(v) => v,
        Err(e) => return Err(attach_logs(e, &session)),
    };

    let output = ir_json::node_from_json(&store, &ir_value)
        .map_err(|m| attach_logs(BuildError::BadOutput(m), &session))?;
    Ok(BuildOutput {
        output,
        deps: std::mem::take(&mut session.deps),
        logs: std::mem::take(&mut session.logs),
    })
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

fn js_error(path: &str, session: SessionState, msg: String) -> BuildError {
    if session.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return BuildError::Cancelled;
    }
    attach_logs(BuildError::Js(format!("{path}: {msg}")), &session)
}

/// Errors keep their message; logs travel with the scheduler separately in
/// the success path, but on error we append them so the agent sees both.
fn attach_logs(err: BuildError, session: &SessionState) -> BuildError {
    if session.logs.is_empty() {
        return err;
    }
    let (msg, rewrap): (String, fn(String) -> BuildError) = match err {
        BuildError::Js(m) => (m, BuildError::Js),
        BuildError::BadOutput(m) => (m, BuildError::BadOutput),
        other => return other,
    };
    let mut out = msg;
    out.push_str("\n--- console output ---");
    for line in &session.logs {
        out.push_str(&format!("\n[{}] {}", line.level, line.message));
    }
    rewrap(out)
}
