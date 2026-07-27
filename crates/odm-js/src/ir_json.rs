//! Converts the IR JSON tree produced by the framework's `toIRNode` into
//! stored `odm_ir::Node`s, validating geometry/subtree handles against the
//! store. Every node is interned; the caller gets the root's hash.

use odm_ir::{Color, Hash, Node, Transform};
use odm_store::{Object, Store};
use serde_json::Value;

/// Intern a JSON node (and its subtree) and return its hash. `{"ref": hex}`
/// with no other keys is an already-stored subtree (from `ctx.invoke`).
pub fn node_from_json(store: &Store, v: &Value) -> Result<Hash, String> {
    let obj = v.as_object().ok_or_else(|| format!("scene node must be an object, got {v}"))?;

    if let Some(r) = obj.get("ref") {
        if obj.len() != 1 {
            return Err(format!("a subtree ref node must have no other keys, got {v}"));
        }
        let hex = r.as_str().ok_or_else(|| format!("ref must be a hash string, got {r}"))?;
        let h = Hash::from_hex(hex).ok_or_else(|| format!("invalid subtree ref {hex:?}"))?;
        return match store.get(h).as_deref() {
            Some(Object::Node(_)) => Ok(h),
            Some(_) => Err(format!("ref {hex} is geometry, not a scene node")),
            None => Err(format!("unknown subtree ref {hex}")),
        };
    }

    let name = match obj.get("name") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => return Err(format!("node name must be a string, got {other}")),
    };

    let transform = match obj.get("matrix") {
        None => Transform::IDENTITY,
        Some(m) => {
            let arr = m.as_array().ok_or("matrix must be an array of 16 numbers")?;
            if arr.len() != 16 {
                return Err(format!("matrix must have 16 numbers, got {}", arr.len()));
            }
            let mut t = [0.0f64; 16];
            for (i, x) in arr.iter().enumerate() {
                t[i] = x.as_f64().ok_or("matrix entries must be numbers")?;
            }
            Transform(t)
        }
    };

    let color = match obj.get("color") {
        None => None,
        Some(c) => {
            let arr = c.as_array().ok_or("color must be [r,g,b,a]")?;
            if arr.len() != 4 {
                return Err(format!("color must have 4 components, got {}", arr.len()));
            }
            let mut v = [0.0f32; 4];
            for (i, x) in arr.iter().enumerate() {
                v[i] = x.as_f64().ok_or("color components must be numbers")? as f32;
            }
            Some(Color { r: v[0], g: v[1], b: v[2], a: v[3] })
        }
    };

    let mesh = match obj.get("geom") {
        None => None,
        Some(Value::String(hex)) => {
            let h = Hash::from_hex(hex).ok_or_else(|| format!("invalid geometry handle {hex:?}"))?;
            match store.get(h).as_deref() {
                Some(Object::Mesh(_)) => Some(h),
                Some(_) => return Err(format!("handle {hex} is not geometry")),
                None => return Err(format!("unknown geometry handle {hex}")),
            }
        }
        Some(other) => return Err(format!("geom must be a hash string, got {other}")),
    };

    let children = match obj.get("children") {
        None => Vec::new(),
        Some(Value::Array(cs)) => {
            cs.iter().map(|c| node_from_json(store, c)).collect::<Result<Vec<_>, _>>()?
        }
        Some(other) => return Err(format!("children must be an array, got {other}")),
    };

    Ok(store.put(Object::Node(Node { name, transform, color, mesh, children })))
}
