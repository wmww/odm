//! The web executor: odm-build's `Executor` implemented against the page's
//! JS. `run_build` pushes a session frame, asks runtime.js to invoke the
//! doohickey's factory + `__odm.runBuild`, and interns the returned IR JSON.
//! While the factory runs, the framework's ops land in the `op_*` exports
//! below (the browser mirror of odm-js `ops.rs`), which read the top frame.
//! Reentrancy is the load-bearing bit: `op_invoke` → scheduler → executor →
//! JS again, nesting frames LIFO exactly like native isolates.

use odm_build::{
    ApiVersion, BuildError, BuildInput, BuildOutput, Executor, FailedBuild, Invoker,
    cascade_value_hash, node_from_json,
};
use odm_ir::{Hash, Transform};
use odm_kernel::{BoolOp, Kernel};
use odm_store::{Dep, LogLevel, LogLine, Store};
use serde::Deserialize;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

const MAX_LOG_LINES: usize = 1000;

#[wasm_bindgen]
extern "C" {
    /// runtime.js: apply determinism, install the version surface, invoke
    /// the factory, run `__odm.runBuild`, restore. Returns a JSON envelope
    /// `{"ok": <ir>}` or `{"error": {"kind": "js"|"internal", "message": …}}`.
    #[wasm_bindgen(js_namespace = ["__odmWeb"], js_name = runBuild, catch)]
    fn js_run_build(path: &str, api: &str, args: &str, decls: &str) -> Result<String, JsValue>;
}

/// One in-progress build's session (the native `SessionState`).
struct Frame {
    kernel: Arc<Kernel>,
    cascade: HashMap<String, Value>,
    deps: Vec<Dep>,
    logs: Vec<LogLine>,
    invoker: Option<Box<dyn Invoker>>,
}

thread_local! {
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    /// path → raw meta export (and errors), from the manifest; the web
    /// stand-in for evaluating the module in `extract_export`.
    static METAS: RefCell<HashMap<String, MetaEntry>> = RefCell::new(HashMap::new());
}

pub struct MetaEntry {
    /// `Some(value)` = the module's `meta` export; `None` = no export.
    pub meta: Option<Value>,
    /// Export-time extraction failure, replayed verbatim.
    pub error: Option<String>,
}

pub fn set_metas(metas: HashMap<String, MetaEntry>) {
    METAS.with(|m| *m.borrow_mut() = metas);
}

/// The `Executor` the scheduler drives. Stateless: sessions live in the
/// thread-local frame stack (single browser thread; `Send + Sync` is
/// vacuous there).
pub struct WebExecutor;

impl Executor for WebExecutor {
    fn run_build(&self, input: BuildInput<'_>) -> Result<BuildOutput, FailedBuild> {
        let store = input.store.clone();
        FRAMES.with(|f| {
            f.borrow_mut().push(Frame {
                kernel: input.kernel.clone(),
                cascade: input.cascade.clone(),
                deps: Vec::new(),
                logs: Vec::new(),
                invoker: input.invoker,
            })
        });
        let result = js_run_build(
            input.path,
            input.api.name(),
            &serde_json::to_string(input.args).expect("args json"),
            &serde_json::to_string(input.decls).expect("decls json"),
        );
        let frame = FRAMES.with(|f| f.borrow_mut().pop()).expect("frame stack");
        let (logs, deps) = (frame.logs, frame.deps);
        let fail = |error: BuildError, logs: Vec<LogLine>, deps: Vec<Dep>| FailedBuild {
            error,
            logs,
            deps,
        };

        let envelope = match result {
            Ok(s) => s,
            Err(e) => {
                let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                return Err(fail(
                    BuildError::Internal(format!("web runtime: {msg}")),
                    logs,
                    deps,
                ));
            }
        };
        let v: Value = match serde_json::from_str(&envelope) {
            Ok(v) => v,
            Err(e) => {
                return Err(fail(
                    BuildError::Internal(format!("bad build envelope: {e}")),
                    logs,
                    deps,
                ));
            }
        };
        if let Some(err) = v.get("error") {
            let message =
                err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error").to_string();
            let kind = err.get("kind").and_then(|k| k.as_str()).unwrap_or("js");
            let error = match kind {
                "internal" => BuildError::Internal(message),
                "badOutput" => BuildError::BadOutput(message),
                _ => BuildError::Js(format!("{}: {message}", input.path)),
            };
            return Err(fail(error, logs, deps));
        }
        let Some(ir) = v.get("ok") else {
            return Err(fail(
                BuildError::Internal("build envelope missing ok/error".into()),
                logs,
                deps,
            ));
        };
        match node_from_json(&store, ir) {
            Ok(output) => Ok(BuildOutput { output, deps, logs }),
            Err(m) => Err(fail(BuildError::BadOutput(m), logs, deps)),
        }
    }

    fn extract_export(
        &self,
        path: &str,
        _code: &str,
        _api: ApiVersion,
        export: &str,
        _kernel: Arc<Kernel>,
        _store: Arc<Store>,
        _timeout: std::time::Duration,
    ) -> Result<Option<Value>, BuildError> {
        if export != "meta" {
            return Err(BuildError::Internal(format!(
                "extract_export({export:?}) is not available in the web runtime"
            )));
        }
        METAS.with(|m| {
            let metas = m.borrow();
            let Some(entry) = metas.get(path) else { return Ok(None) };
            if let Some(err) = &entry.error {
                // Replay the export-time failure with its original text.
                return Err(BuildError::Js(err.clone()));
            }
            Ok(entry.meta.clone())
        })
    }
}

// ---------- ops (mirror of odm-js ops.rs, JS-glue-facing) ----------

fn with_frame<R>(f: impl FnOnce(&mut Frame) -> Result<R, String>) -> Result<R, JsError> {
    FRAMES
        .with(|frames| {
            let mut frames = frames.borrow_mut();
            let frame = frames
                .last_mut()
                .ok_or_else(|| "ODM engine ops unavailable: no build in progress".to_string())?;
            f(frame)
        })
        .map_err(|e| JsError::new(&e))
}

fn parse_hash(s: &str) -> Result<Hash, String> {
    Hash::from_hex(s).ok_or_else(|| format!("invalid geometry handle {s:?}"))
}

#[derive(Deserialize)]
struct Operand {
    geom: String,
    matrix: [f64; 16],
}

fn operands(json: &str) -> Result<Vec<(Hash, Transform)>, String> {
    let ops: Vec<Operand> = serde_json::from_str(json).map_err(|e| format!("bad operands: {e}"))?;
    ops.iter().map(|o| Ok((parse_hash(&o.geom)?, Transform(o.matrix)))).collect()
}

fn arr3(v: &[f64], what: &str) -> Result<[f64; 3], String> {
    <[f64; 3]>::try_from(v).map_err(|_| format!("{what} must have 3 numbers, got {}", v.len()))
}

#[wasm_bindgen]
pub fn op_solid_box(size: &[f64], center: bool) -> Result<String, JsError> {
    with_frame(|f| {
        let size = arr3(size, "box size")?;
        let h = f.kernel.cube(size[0], size[1], size[2], center).map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_solid_cylinder(
    height: f64,
    r1: f64,
    r2: f64,
    segments: f64,
    center: bool,
) -> Result<String, JsError> {
    with_frame(|f| {
        let h = f
            .kernel
            .cylinder(height, r1, r2, segments as i32, center)
            .map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_solid_sphere(radius: f64, segments: f64) -> Result<String, JsError> {
    with_frame(|f| {
        let h = f.kernel.sphere(radius, segments as i32).map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_solid_extrude(
    polygons_json: &str,
    height: f64,
    slices: f64,
    twist_degrees: f64,
    scale_top: &[f64],
) -> Result<String, JsError> {
    with_frame(|f| {
        let polygons: Vec<Vec<[f64; 2]>> =
            serde_json::from_str(polygons_json).map_err(|e| format!("bad polygons: {e}"))?;
        if scale_top.len() != 2 {
            return Err("scale_top must have 2 numbers".into());
        }
        let h = f
            .kernel
            .extrude(&polygons, height, slices as i32, twist_degrees, [scale_top[0], scale_top[1]])
            .map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_solid_revolve(polygons_json: &str, segments: f64, degrees: f64) -> Result<String, JsError> {
    with_frame(|f| {
        let polygons: Vec<Vec<[f64; 2]>> =
            serde_json::from_str(polygons_json).map_err(|e| format!("bad polygons: {e}"))?;
        let h = f.kernel.revolve(&polygons, segments as i32, degrees).map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_solid_from_mesh(positions: &[f64], indices: &[u32]) -> Result<String, JsError> {
    with_frame(|f| {
        let h = f.kernel.solid_from_mesh(positions, indices).map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_boolean(kind: &str, operands_json: &str) -> Result<String, JsError> {
    with_frame(|f| {
        let op = match kind {
            "union" => BoolOp::Union,
            "difference" => BoolOp::Difference,
            "intersection" => BoolOp::Intersection,
            other => return Err(format!("unknown boolean op {other:?}")),
        };
        let args = operands(operands_json)?;
        let h = f.kernel.boolean(op, &args, None).map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_hull(operands_json: &str) -> Result<String, JsError> {
    with_frame(|f| {
        let args = operands(operands_json)?;
        let h = f.kernel.hull(&args, None).map_err(|e| e.to_string())?;
        Ok(h.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_transform_bake(geom: &str, matrix: &[f64]) -> Result<String, JsError> {
    with_frame(|f| {
        let matrix: [f64; 16] =
            matrix.try_into().map_err(|_| format!("matrix must have 16 numbers, got {}", matrix.len()))?;
        let h = parse_hash(geom)?;
        let out = f.kernel.transform_solid(h, Transform(matrix), None).map_err(|e| e.to_string())?;
        Ok(out.to_hex())
    })
}

#[wasm_bindgen]
pub fn op_volume(geom: &str) -> Result<f64, JsError> {
    with_frame(|f| f.kernel.volume(parse_hash(geom)?).map_err(|e| e.to_string()))
}

#[wasm_bindgen]
pub fn op_area(geom: &str) -> Result<f64, JsError> {
    with_frame(|f| f.kernel.surface_area(parse_hash(geom)?).map_err(|e| e.to_string()))
}

#[wasm_bindgen]
pub fn op_bounds(geom: &str) -> Result<String, JsError> {
    with_frame(|f| {
        let b = f.kernel.bounds(parse_hash(geom)?).map_err(|e| e.to_string())?;
        let v = match b {
            Some(b) => serde_json::json!({ "min": b.min, "max": b.max }),
            None => Value::Null,
        };
        Ok(v.to_string())
    })
}

#[wasm_bindgen]
pub fn op_raycast(geom: &str, origin: &[f64], dir: &[f64], max_dist: f64) -> Result<String, JsError> {
    with_frame(|f| {
        let origin = arr3(origin, "origin")?;
        let dir = arr3(dir, "dir")?;
        let hit = f.kernel.raycast(parse_hash(geom)?, origin, dir, max_dist).map_err(|e| e.to_string())?;
        let v = match hit {
            Some(h) =>

                serde_json::json!({ "distance": h.distance, "position": h.position, "normal": h.normal }),
            None => Value::Null,
        };
        Ok(v.to_string())
    })
}

#[wasm_bindgen]
pub fn op_clearance(a: &str, b: &str) -> Result<String, JsError> {
    with_frame(|f| {
        let (a, b) = (parse_hash(a)?, parse_hash(b)?);
        let c = f
            .kernel
            .clearance(&[(a, Transform::IDENTITY)], &[(b, Transform::IDENTITY)], None)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({ "overlap": c.overlap, "gap_lower_bound": c.gap_lower_bound })
            .to_string())
    })
}

#[wasm_bindgen]
pub fn op_cascade_read(key: &str) -> Result<String, JsError> {
    with_frame(|f| {
        let value = f.cascade.get(key).cloned();
        let value_hash = cascade_value_hash(value.as_ref());
        if !f.deps.iter().any(|d| matches!(d, Dep::Cascade { key: k, .. } if k == key)) {
            f.deps.push(Dep::Cascade { key: key.to_string(), value: value_hash });
        }
        let v = match value {
            Some(v) => serde_json::json!({ "present": true, "value": v }),
            None => serde_json::json!({ "present": false, "value": Value::Null }),
        };
        Ok(v.to_string())
    })
}

/// Reentrant: the nested build's ops fire while this is on the stack.
#[wasm_bindgen]
pub fn op_invoke(path: &str, args_json: &str, cascade_json: &str) -> Result<String, JsError> {
    let args: Value =
        serde_json::from_str(args_json).map_err(|e| JsError::new(&format!("bad invoke args: {e}")))?;
    let cascade: Value = serde_json::from_str(cascade_json)
        .map_err(|e| JsError::new(&format!("bad invoke cascade: {e}")))?;
    let cascade = match cascade {
        Value::Object(m) => m,
        Value::Null => serde_json::Map::new(),
        _ => return Err(JsError::new("invoke cascade must be an object")),
    };
    // Take the invoker out for the nested build (which pushes its own
    // frames); the borrow must not be held across it.
    let mut invoker = with_frame(|f| {
        f.invoker.take().ok_or_else(|| "ctx.invoke is not available in this build".to_string())
    })?;
    let result = invoker.invoke(path, &args, &cascade);
    let path = path.to_string();
    with_frame(move |f| {
        f.invoker = Some(invoker);
        match result {
            Ok(output) => {
                f.deps.push(Dep::Invoke {
                    path,
                    args,
                    cascade,
                    outcome: odm_store::InvokeOutcome::Output(output),
                });
                Ok(output.to_hex())
            }
            Err(e) => {
                // The failure is part of this build's inputs even when the
                // throw below is caught (see odm-js ops.rs).
                if let Some(identity) = e.identity {
                    f.deps.push(Dep::Invoke {
                        path: path.clone(),
                        args,
                        cascade,
                        outcome: odm_store::InvokeOutcome::Failure(identity),
                    });
                }
                Err(format!("invoke({path:?}) failed: {}", e.message))
            }
        }
    })
}

#[wasm_bindgen]
pub fn op_log(level: &str, message: &str) {
    // Outside a build (framework globals persist between builds on web),
    // console output just goes nowhere — same as a dropped isolate.
    FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        let Some(f) = frames.last_mut() else { return };
        if f.logs.len() < MAX_LOG_LINES {
            f.logs.push(LogLine { level: LogLevel::parse(level), message: message.to_string() });
        } else if f.logs.len() == MAX_LOG_LINES {
            f.logs.push(LogLine {
                level: LogLevel::Warn,
                message: format!("(console output truncated at {MAX_LOG_LINES} lines)"),
            });
        }
    });
}
