use crate::session::SessionState;
use odm_build::cascade_value_hash;
use deno_core::{OpState, op2};
use deno_error::JsErrorBox;
use odm_ir::{Hash, Transform};
use odm_kernel::{BoolOp, KernelError};
use odm_store::Dep;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_LOG_LINES: usize = 1000;

fn sess(state: &mut OpState) -> &mut SessionState {
    state.borrow_mut::<SessionState>()
}

fn kerr(e: KernelError) -> JsErrorBox {
    JsErrorBox::generic(e.to_string())
}

fn parse_hash(s: &str) -> Result<Hash, JsErrorBox> {
    Hash::from_hex(s).ok_or_else(|| JsErrorBox::type_error(format!("invalid geometry handle {s:?}")))
}

#[derive(Deserialize)]
struct Operand {
    geom: String,
    matrix: [f64; 16],
}

fn operands(ops: &[Operand]) -> Result<Vec<(Hash, Transform)>, JsErrorBox> {
    ops.iter().map(|o| Ok((parse_hash(&o.geom)?, Transform(o.matrix)))).collect()
}

fn arr3(v: &[f64], what: &str) -> Result<[f64; 3], JsErrorBox> {
    <[f64; 3]>::try_from(v)
        .map_err(|_| JsErrorBox::type_error(format!("{what} must have 3 numbers, got {}", v.len())))
}

fn arr16(v: &[f64]) -> Result<[f64; 16], JsErrorBox> {
    <[f64; 16]>::try_from(v)
        .map_err(|_| JsErrorBox::type_error(format!("matrix must have 16 numbers, got {}", v.len())))
}

// ---------- solids ----------

#[op2]
#[string]
pub fn op_solid_box(
    state: &mut OpState,
    #[serde] size: Vec<f64>,
    center: bool,
) -> Result<String, JsErrorBox> {
    let size = arr3(&size, "box size")?;
    let s = sess(state);
    let h = s.kernel.cube(size[0], size[1], size[2], center).map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_solid_cylinder(
    state: &mut OpState,
    height: f64,
    r1: f64,
    r2: f64,
    segments: f64,
    center: bool,
) -> Result<String, JsErrorBox> {
    let s = sess(state);
    let h = s.kernel.cylinder(height, r1, r2, segments as i32, center).map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_solid_sphere(state: &mut OpState, radius: f64, segments: f64) -> Result<String, JsErrorBox> {
    let s = sess(state);
    let h = s.kernel.sphere(radius, segments as i32).map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_solid_extrude(
    state: &mut OpState,
    #[serde] polygons: Vec<Vec<[f64; 2]>>,
    height: f64,
    slices: f64,
    twist_degrees: f64,
    #[serde] scale_top: Vec<f64>,
) -> Result<String, JsErrorBox> {
    if scale_top.len() != 2 {
        return Err(JsErrorBox::type_error("scale_top must have 2 numbers"));
    }
    let s = sess(state);
    let h = s
        .kernel
        .extrude(&polygons, height, slices as i32, twist_degrees, [scale_top[0], scale_top[1]])
        .map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_solid_revolve(
    state: &mut OpState,
    #[serde] polygons: Vec<Vec<[f64; 2]>>,
    segments: f64,
    degrees: f64,
) -> Result<String, JsErrorBox> {
    let s = sess(state);
    let h = s.kernel.revolve(&polygons, segments as i32, degrees).map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_solid_sweep(
    state: &mut OpState,
    #[serde] polygons: Vec<Vec<[f64; 2]>>,
    #[serde] frames: Vec<[f64; 12]>,
) -> Result<String, JsErrorBox> {
    let s = sess(state);
    let h = s.kernel.sweep(&polygons, &frames).map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_solid_from_mesh(
    state: &mut OpState,
    #[buffer] positions: &[f64],
    #[buffer] indices: &[u32],
) -> Result<String, JsErrorBox> {
    let s = sess(state);
    let h = s.kernel.solid_from_mesh(positions, indices).map_err(kerr)?;
    Ok(h.to_hex())
}

// ---------- csg ----------

#[op2]
#[string]
pub fn op_boolean(
    state: &mut OpState,
    #[string] kind: &str,
    #[serde] ops_in: Vec<Operand>,
) -> Result<String, JsErrorBox> {
    let op = match kind {
        "union" => BoolOp::Union,
        "difference" => BoolOp::Difference,
        "intersection" => BoolOp::Intersection,
        other => return Err(JsErrorBox::type_error(format!("unknown boolean op {other:?}"))),
    };
    let args = operands(&ops_in)?;
    let s = sess(state);
    let h = s.kernel.boolean(op, &args, s.cancel.as_ref()).map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_hull(state: &mut OpState, #[serde] ops_in: Vec<Operand>) -> Result<String, JsErrorBox> {
    let args = operands(&ops_in)?;
    let s = sess(state);
    let h = s.kernel.hull(&args, s.cancel.as_ref()).map_err(kerr)?;
    Ok(h.to_hex())
}

#[op2]
#[string]
pub fn op_transform_bake(
    state: &mut OpState,
    #[string] geom: &str,
    #[serde] matrix: Vec<f64>,
) -> Result<String, JsErrorBox> {
    let h = parse_hash(geom)?;
    let matrix = arr16(&matrix)?;
    let s = sess(state);
    let out = s.kernel.transform_solid(h, Transform(matrix), s.cancel.as_ref()).map_err(kerr)?;
    Ok(out.to_hex())
}

// ---------- queries ----------

#[op2(fast)]
pub fn op_volume(state: &mut OpState, #[string] geom: &str) -> Result<f64, JsErrorBox> {
    let h = parse_hash(geom)?;
    sess(state).kernel.volume(h).map_err(kerr)
}

#[op2(fast)]
pub fn op_area(state: &mut OpState, #[string] geom: &str) -> Result<f64, JsErrorBox> {
    let h = parse_hash(geom)?;
    sess(state).kernel.surface_area(h).map_err(kerr)
}

#[derive(Serialize)]
struct BoundsJson {
    min: [f64; 3],
    max: [f64; 3],
}

#[op2]
#[serde]
pub fn op_bounds(state: &mut OpState, #[string] geom: &str) -> Result<Option<BoundsJson>, JsErrorBox> {
    let h = parse_hash(geom)?;
    let b = sess(state).kernel.bounds(h).map_err(kerr)?;
    Ok(b.map(|b| BoundsJson { min: b.min, max: b.max }))
}

#[derive(Serialize)]
struct RayHitJson {
    distance: f64,
    position: [f64; 3],
    normal: [f64; 3],
}

#[op2]
#[serde]
pub fn op_raycast(
    state: &mut OpState,
    #[string] geom: &str,
    #[serde] origin: Vec<f64>,
    #[serde] dir: Vec<f64>,
    max_dist: f64,
) -> Result<Option<RayHitJson>, JsErrorBox> {
    let h = parse_hash(geom)?;
    let origin = arr3(&origin, "origin")?;
    let dir = arr3(&dir, "dir")?;
    let hit = sess(state).kernel.raycast(h, origin, dir, max_dist).map_err(kerr)?;
    Ok(hit.map(|h| RayHitJson { distance: h.distance, position: h.position, normal: h.normal }))
}

#[derive(Serialize)]
struct ClearanceJson {
    distance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    closest: Option<[[f64; 3]; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    separate: Option<[f64; 3]>,
}

#[op2]
#[serde]
pub fn op_clearance(
    state: &mut OpState,
    #[string] a: &str,
    #[string] b: &str,
) -> Result<ClearanceJson, JsErrorBox> {
    let (a, b) = (parse_hash(a)?, parse_hash(b)?);
    let s = sess(state);
    let c = s
        .kernel
        .clearance(&[(a, Transform::IDENTITY)], &[(b, Transform::IDENTITY)], s.cancel.as_ref())
        .map_err(kerr)?;
    Ok(ClearanceJson { distance: c.distance, closest: c.closest, separate: c.separate })
}

// ---------- cascade / invoke / log ----------

#[derive(Serialize)]
struct CtxRead {
    present: bool,
    value: Value,
}

#[op2]
#[serde]
pub fn op_cascade_read(state: &mut OpState, #[string] key: String) -> CtxRead {
    let s = sess(state);
    let value = s.cascade.get(&key).cloned();
    let value_hash = cascade_value_hash(value.as_ref());
    // Record each key once; within a build the value cannot change.
    if !s.deps.iter().any(|d| matches!(d, Dep::Cascade { key: k, .. } if *k == key)) {
        s.deps.push(Dep::Cascade { key, value: value_hash });
    }
    match value {
        Some(v) => CtxRead { present: true, value: v },
        None => CtxRead { present: false, value: Value::Null },
    }
}

// Reentrant: the nested build's ops fire while op_invoke is on the stack.
#[op2(reentrant)]
#[string]
pub fn op_invoke(
    state: &mut OpState,
    #[string] path: String,
    #[serde] args: serde_json::Value,
    #[serde] cascade: serde_json::Value,
) -> Result<String, JsErrorBox> {
    let cascade = match cascade {
        serde_json::Value::Object(m) => m,
        serde_json::Value::Null => serde_json::Map::new(),
        _ => return Err(JsErrorBox::type_error("invoke cascade must be an object")),
    };
    let s = sess(state);
    let Some(mut invoker) = s.invoker.take() else {
        return Err(JsErrorBox::generic("ctx.invoke is not available in this build"));
    };
    // The nested build runs on this thread with its own isolate (LIFO).
    let result = invoker.invoke(&path, &args, &cascade);
    let s = sess(state);
    s.invoker = Some(invoker);
    match result {
        Ok(output) => {
            s.deps.push(Dep::Invoke {
                path,
                args,
                cascade,
                outcome: odm_store::InvokeOutcome::Output(output),
            });
            Ok(output.to_hex())
        }
        Err(e) => {
            // The failure is part of this build's inputs even when the throw
            // below is caught: without the dep, a memoized fallback output
            // would keep validating after the child is fixed.
            if let Some(identity) = e.identity {
                s.deps.push(Dep::Invoke {
                    path: path.clone(),
                    args,
                    cascade,
                    outcome: odm_store::InvokeOutcome::Failure(identity),
                });
            }
            Err(JsErrorBox::generic(format!("invoke({path:?}) failed: {}", e.message)))
        }
    }
}

#[op2(fast)]
pub fn op_log(state: &mut OpState, #[string] level: &str, #[string] message: &str) {
    let s = sess(state);
    if s.logs.len() < MAX_LOG_LINES {
        s.logs.push(crate::session::LogLine {
            level: crate::session::LogLevel::parse(level),
            message: message.to_string(),
        });
    } else if s.logs.len() == MAX_LOG_LINES {
        s.logs.push(crate::session::LogLine {
            level: crate::session::LogLevel::Warn,
            message: format!("(console output truncated at {MAX_LOG_LINES} lines)"),
        });
    }
}

deno_core::extension!(
    odm_ops,
    ops = [
        op_solid_box,
        op_solid_cylinder,
        op_solid_sphere,
        op_solid_extrude,
        op_solid_revolve,
        op_solid_sweep,
        op_solid_from_mesh,
        op_boolean,
        op_hull,
        op_transform_bake,
        op_volume,
        op_area,
        op_bounds,
        op_raycast,
        op_clearance,
        op_cascade_read,
        op_invoke,
        op_log,
    ],
);
