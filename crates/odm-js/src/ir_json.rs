//! Converts the IR JSON tree produced by the framework's `toIRNode` into
//! `odm_ir::Node`, validating geometry handles against the store.

use odm_ir::{Color, Hash, Node, Transform};
use odm_store::{Object, Store};
use serde_json::Value;

pub fn node_from_json(store: &Store, v: &Value) -> Result<Node, String> {
    let obj = v.as_object().ok_or_else(|| format!("scene node must be an object, got {v}"))?;

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

    Ok(Node { name, transform, color, mesh, children })
}

/// The reverse direction: IR node → JSON, used when a nested invoke's output
/// is embedded into the calling build's tree.
pub fn node_to_json(node: &Node) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(n) = &node.name {
        obj.insert("name".into(), Value::String(n.clone()));
    }
    if !node.transform.is_identity() {
        obj.insert(
            "matrix".into(),
            Value::Array(node.transform.0.iter().map(|&x| x.into()).collect()),
        );
    }
    if let Some(c) = node.color {
        obj.insert(
            "color".into(),
            Value::Array([c.r, c.g, c.b, c.a].iter().map(|&x| (x as f64).into()).collect()),
        );
    }
    if let Some(m) = node.mesh {
        obj.insert("geom".into(), Value::String(m.to_hex()));
    }
    if !node.children.is_empty() {
        obj.insert("children".into(), Value::Array(node.children.iter().map(node_to_json).collect()));
    }
    Value::Object(obj)
}
