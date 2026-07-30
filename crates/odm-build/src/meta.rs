//! Doohickey metadata: `export const meta = { inputs: {...}, presets: {...} }`.
//!
//! `meta.inputs` is ONE map, name → entry; an entry is a profiled JSON
//! Schema (`type`, `enum`, `default`, `description`, `minimum`/`maximum`,
//! `items`, `properties`/`required`) plus the ODM key `cascade`. Unknown
//! keys are rejected by our own allowlist walk — which is also where ODM
//! keys are recognized, so a typo'd key errors instead of silently changing
//! an input's kind. ODM keys are stripped and extension types desugared
//! before value validation, which is delegated to the `jsonschema` crate.
//! Metadata is read by evaluating the module (no build); callers cache the
//! result by code hash (see `BuildEngine::meta`).

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// ODM's extension types: `type` values beyond JSON Schema's own. On the
/// wire they are canonical JSON (vectors `[x, y, z]`, matrix4 = 16 numbers
/// column-major, color = hex string or `[r, g, b]`); `ctx.input` hydrates them
/// into real THREE instances, declaration-driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtType {
    Solid,
    Vector2,
    Vector3,
    Quaternion,
    Matrix4,
    Color,
}

impl ExtType {
    pub fn parse(s: &str) -> Option<ExtType> {
        Some(match s {
            "solid" => ExtType::Solid,
            "vector2" => ExtType::Vector2,
            "vector3" => ExtType::Vector3,
            "quaternion" => ExtType::Quaternion,
            "matrix4" => ExtType::Matrix4,
            "color" => ExtType::Color,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            ExtType::Solid => "solid",
            ExtType::Vector2 => "vector2",
            ExtType::Vector3 => "vector3",
            ExtType::Quaternion => "quaternion",
            ExtType::Matrix4 => "matrix4",
            ExtType::Color => "color",
        }
    }

    /// Standard-JSON-Schema equivalent, for value validation.
    fn desugar(self) -> Value {
        fn nums(n: usize) -> Value {
            json!({ "type": "array", "items": { "type": "number" }, "minItems": n, "maxItems": n })
        }
        match self {
            // A Solid crosses boundaries as the framework's handle tag.
            ExtType::Solid => json!({
                "type": "object",
                "properties": { "__odm_solid__": { "const": true }, "geom": { "type": "string" } },
                "required": ["__odm_solid__", "geom"],
            }),
            ExtType::Vector2 => nums(2),
            ExtType::Vector3 => nums(3),
            ExtType::Quaternion => nums(4),
            ExtType::Matrix4 => nums(16),
            ExtType::Color => json!({ "anyOf": [ { "type": "string" }, nums(3) ] }),
        }
    }

    /// Canonicalize the serialized-THREE-instance forms (`{x, y, z}`,
    /// `{elements: [...]}`, `{r, g, b}`) to the wire form. Anything else
    /// passes through untouched and validation reports it.
    pub fn normalize(self, v: &Value) -> Value {
        let get = |o: &Map<String, Value>, keys: &[&str]| -> Option<Value> {
            keys.iter().find_map(|k| o.get(*k)).filter(|v| v.is_number()).cloned()
        };
        let Some(o) = v.as_object() else { return v.clone() };
        let fields: &[&[&str]] = match self {
            ExtType::Vector2 => &[&["x"], &["y"]],
            ExtType::Vector3 => &[&["x"], &["y"], &["z"]],
            ExtType::Quaternion => &[&["x", "_x"], &["y", "_y"], &["z", "_z"], &["w", "_w"]],
            ExtType::Color => &[&["r"], &["g"], &["b"]],
            ExtType::Matrix4 => {
                return match o.get("elements") {
                    Some(e @ Value::Array(a)) if a.len() == 16 => e.clone(),
                    _ => v.clone(),
                };
            }
            ExtType::Solid => return v.clone(),
        };
        match fields.iter().map(|keys| get(o, keys)).collect::<Option<Vec<Value>>>() {
            Some(parts) => Value::Array(parts),
            None => v.clone(),
        }
    }
}

/// One declared input.
#[derive(Debug)]
pub struct Input {
    /// `cascade: true`: resolved up the invoke chain — nearest explicitly
    /// provided value, view outermost; `default` mandatory. Otherwise the value
    /// comes from the immediate caller's args, falling back to `default`
    /// (no default = required).
    pub cascade: bool,
    pub extension: Option<ExtType>,
    /// The entry as authored (with `default` normalized), for `odm interface`.
    pub authored: Map<String, Value>,
    /// Normalized default, if declared.
    pub default: Option<Value>,
    validator: jsonschema::Validator,
}

impl Input {
    /// Normalize a value for this input (THREE-instance forms → wire form)
    /// and validate it against the schema.
    pub fn accept(&self, name: &str, value: &Value) -> Result<Value, String> {
        let v = match self.extension {
            Some(e) => e.normalize(value),
            None => value.clone(),
        };
        match self.validator.validate(&v) {
            Ok(()) => Ok(v),
            Err(e) => {
                let ty = self
                    .authored
                    .get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| format!(" (declared type: {t})"))
                    .unwrap_or_default();
                Err(format!("input {name:?}: {e}{ty}"))
            }
        }
    }

    /// The `type` as authored, if any (extension names included).
    pub fn type_name(&self) -> Option<&str> {
        self.authored.get("type").and_then(|t| t.as_str())
    }

    pub fn minimum(&self) -> Option<f64> {
        self.authored.get("minimum").and_then(|v| v.as_f64())
    }

    pub fn maximum(&self) -> Option<f64> {
        self.authored.get("maximum").and_then(|v| v.as_f64())
    }
}

/// A doohickey's parsed `export const meta`. Absent export = empty meta.
#[derive(Debug, Default)]
pub struct Meta {
    pub inputs: BTreeMap<String, Input>,
    /// Named input bundles: preset name → { input name → normalized value }.
    pub presets: BTreeMap<String, Map<String, Value>>,
}

impl Meta {
    /// Validate and index a raw `meta` export.
    pub fn parse(raw: &Value) -> Result<Meta, String> {
        let Some(top) = raw.as_object() else {
            return Err("meta must be an object: { inputs?, presets? }".into());
        };
        for key in top.keys() {
            if key != "inputs" && key != "presets" {
                return Err(format!("unknown meta key {key:?}; allowed: inputs, presets"));
            }
        }

        let mut inputs = BTreeMap::new();
        if let Some(raw_inputs) = top.get("inputs") {
            let Some(raw_inputs) = raw_inputs.as_object() else {
                return Err("meta.inputs must be an object: { name: {..schema..} }".into());
            };
            for (name, entry) in raw_inputs {
                let input = parse_input(name, entry).map_err(|e| format!("meta.inputs: {e}"))?;
                inputs.insert(name.clone(), input);
            }
        }

        let mut presets = BTreeMap::new();
        if let Some(raw_presets) = top.get("presets") {
            let Some(raw_presets) = raw_presets.as_object() else {
                return Err("meta.presets must be an object: { name: { input: value } }".into());
            };
            for (preset, values) in raw_presets {
                let Some(values) = values.as_object() else {
                    return Err(format!(
                        "meta.presets[{preset:?}] must be an object: {{ input: value }}"
                    ));
                };
                let mut normalized = Map::new();
                for (input_name, value) in values {
                    let Some(input) = inputs.get(input_name) else {
                        return Err(format!(
                            "meta.presets[{preset:?}] sets {input_name:?}, which is not in meta.inputs"
                        ));
                    };
                    let v = input
                        .accept(input_name, value)
                        .map_err(|e| format!("meta.presets[{preset:?}]: {e}"))?;
                    normalized.insert(input_name.clone(), v);
                }
                presets.insert(preset.clone(), normalized);
            }
        }

        Ok(Meta { inputs, presets })
    }
}

const STANDARD_TYPES: &[&str] =
    &["number", "integer", "string", "boolean", "array", "object", "null"];

fn parse_input(name: &str, entry: &Value) -> Result<Input, String> {
    let Some(entry) = entry.as_object() else {
        return Err(format!("{name:?} must be an object (a profiled JSON Schema)"));
    };
    let extension = check_schema(&format!("{name:?}"), entry, true)?;

    let cascade = match entry.get("cascade") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(format!("{name:?}: cascade must be true or false")),
    };

    // Build the value validator from the desugared, ODM-keys-stripped schema.
    let mut schema = entry.clone();
    schema.remove("cascade");
    schema.remove("default");
    let schema = match extension {
        Some(e) => e.desugar(),
        None => Value::Object(schema),
    };
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| format!("{name:?}: schema does not compile: {e}"))?;

    let mut authored = entry.clone();
    let mut input = Input { cascade, extension, authored: Map::new(), default: None, validator };

    if let Some(raw_default) = entry.get("default") {
        if extension == Some(ExtType::Solid) {
            return Err(format!("{name:?}: solid inputs cannot have a default"));
        }
        let d = input.accept(name, raw_default).map_err(|e| format!("bad default for {e}"))?;
        authored.insert("default".into(), d.clone());
        input.default = Some(d);
    } else if cascade {
        return Err(format!("{name:?}: a cascade input must declare a default"));
    }
    input.authored = authored;
    Ok(input)
}

/// The profile allowlist walk. Returns the extension type if `type` names
/// one (top level only). `top` also admits the ODM keys (`cascade`) and
/// `default`, which have no meaning in nested schemas.
fn check_schema(at: &str, entry: &Map<String, Value>, top: bool) -> Result<Option<ExtType>, String> {
    const NESTED_KEYS: &[&str] =
        &["type", "enum", "description", "minimum", "maximum", "items", "properties", "required"];
    for key in entry.keys() {
        let ok = NESTED_KEYS.contains(&key.as_str())
            || (top && (key == "cascade" || key == "default"));
        if !ok {
            return Err(format!(
                "{at}: unknown key {key:?}; allowed: {}{}",
                NESTED_KEYS.join(", "),
                if top { ", default, cascade" } else { "" }
            ));
        }
    }

    // No `type` (and no `enum`) = any JSON value; useful for pass-through
    // inputs. Unknown *keys* are still rejected above.
    let ty = match entry.get("type") {
        None => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => return Err(format!("{at}: `type` must be a string")),
    };
    let extension = match ty {
        Some(t) if STANDARD_TYPES.contains(&t) => None,
        Some(t) => match ExtType::parse(t) {
            Some(e) if top => Some(e),
            Some(_) => {
                return Err(format!(
                    "{at}: extension type {t:?} is only supported at the top level of an input"
                ));
            }
            None => {
                return Err(format!(
                    "{at}: unknown type {t:?}; JSON Schema types: {}; ODM types: \
                     solid, vector2, vector3, quaternion, matrix4, color",
                    STANDARD_TYPES.join(", ")
                ));
            }
        },
        None => None,
    };
    if extension.is_some() {
        for key in ["enum", "minimum", "maximum", "items", "properties", "required"] {
            if entry.contains_key(key) {
                return Err(format!(
                    "{at}: {key:?} cannot be combined with the extension type {:?}",
                    extension.unwrap().name()
                ));
            }
        }
        return Ok(extension);
    }

    if let Some(d) = entry.get("description")
        && !d.is_string()
    {
        return Err(format!("{at}: description must be a string"));
    }
    if let Some(e) = entry.get("enum") {
        match e.as_array() {
            Some(a) if !a.is_empty() => {}
            _ => return Err(format!("{at}: enum must be a non-empty array")),
        }
    }
    for key in ["minimum", "maximum"] {
        if let Some(v) = entry.get(key) {
            if !v.is_number() {
                return Err(format!("{at}: {key} must be a number"));
            }
            if !matches!(ty, Some("number" | "integer")) {
                return Err(format!("{at}: {key} needs type number or integer"));
            }
        }
    }
    if let Some(items) = entry.get("items") {
        if ty != Some("array") {
            return Err(format!("{at}: items needs type \"array\""));
        }
        let Some(items) = items.as_object() else {
            return Err(format!("{at}: items must be a schema object"));
        };
        check_schema(&format!("{at}.items"), items, false)?;
    }
    if let Some(props) = entry.get("properties") {
        if ty != Some("object") {
            return Err(format!("{at}: properties needs type \"object\""));
        }
        let Some(props) = props.as_object() else {
            return Err(format!("{at}: properties must be an object of schemas"));
        };
        for (pname, pschema) in props {
            let Some(pschema) = pschema.as_object() else {
                return Err(format!("{at}.properties[{pname:?}] must be a schema object"));
            };
            check_schema(&format!("{at}.{pname}"), pschema, false)?;
        }
    }
    if let Some(req) = entry.get("required") {
        if ty != Some("object") {
            return Err(format!("{at}: required needs type \"object\""));
        }
        let Some(req) = req.as_array() else {
            return Err(format!("{at}: required must be an array of property names"));
        };
        let props = entry.get("properties").and_then(|p| p.as_object());
        for r in req {
            let Some(r) = r.as_str() else {
                return Err(format!("{at}: required entries must be strings"));
            };
            if let Some(props) = props
                && !props.contains_key(r)
            {
                return Err(format!("{at}: required names {r:?}, which is not in properties"));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_three_instance_forms() {
        let v = ExtType::Vector3.normalize(&json!({"x": 1.0, "y": 2.0, "z": 3.0}));
        assert_eq!(v, json!([1.0, 2.0, 3.0]));
        let q = ExtType::Quaternion.normalize(&json!({"_x": 0.0, "_y": 0.0, "_z": 0.0, "_w": 1.0}));
        assert_eq!(q, json!([0.0, 0.0, 0.0, 1.0]));
        let m = ExtType::Matrix4.normalize(&json!({"elements": (vec![1.0f64; 16])}));
        assert_eq!(m.as_array().map(|a| a.len()), Some(16));
        let c = ExtType::Color.normalize(&json!({"isColor": true, "r": 1.0, "g": 0.5, "b": 0.0}));
        assert_eq!(c, json!([1.0, 0.5, 0.0]));
        // Canonical forms pass through.
        assert_eq!(ExtType::Vector2.normalize(&json!([4, 5])), json!([4, 5]));
        // Junk passes through for validation to report.
        assert_eq!(ExtType::Vector3.normalize(&json!("no")), json!("no"));
    }

    fn parse(v: Value) -> Result<Meta, String> {
        Meta::parse(&v)
    }

    #[test]
    fn accepts_a_full_meta() {
        let m = parse(json!({
            "inputs": {
                "width": { "type": "number", "default": 60, "minimum": 1, "description": "outer width" },
                "t": { "type": "number", "cascade": true, "default": 0, "minimum": 0, "maximum": 2 },
                "finish": { "enum": ["raw", "painted"], "default": "raw" },
                "cutter": { "type": "solid" },
                "offset": { "type": "vector3", "default": [0, 0, 0] },
            },
            "presets": {
                "tall": { "width": 90, "finish": "painted" },
            },
        }))
        .unwrap();
        assert_eq!(m.inputs.len(), 5);
        assert!(m.inputs["t"].cascade);
        assert_eq!(m.inputs["offset"].extension, Some(ExtType::Vector3));
        assert_eq!(m.inputs["cutter"].default, None);
        assert_eq!(m.presets["tall"]["width"], json!(90));
    }

    #[test]
    fn rejects_bad_shapes() {
        let cases: Vec<(Value, &str)> = vec![
            (json!(7), "must be an object"),
            (json!({"input": {}}), "unknown meta key"),
            (json!({"inputs": {"w": {"type": "number", "minimun": 3}}}), "unknown key \"minimun\""),
            (json!({"inputs": {"w": {"type": "number", "cascadee": true}}}), "unknown key \"cascadee\""),
            (json!({"inputs": {"w": {"type": "float"}}}), "unknown type \"float\""),
            (json!({"inputs": {"t": {"type": "number", "cascade": true}}}), "must declare a default"),
            (json!({"inputs": {"w": {"type": "number", "default": "wide"}}}), "bad default"),
            (json!({"inputs": {"v": {"type": "vector3", "default": [1, 2]}}}), "bad default"),
            (json!({"inputs": {"s": {"type": "solid", "default": 1}}}), "cannot have a default"),
            (json!({"inputs": {"s": {"type": "string", "minimum": 0}}}), "needs type number"),
            (json!({"inputs": {"a": {"type": "array", "items": {"type": "vector3"}}}}), "only supported at the top level"),
            (json!({"inputs": {"o": {"type": "object", "properties": {"x": {"type": "number", "oops": 1}}}}}), "unknown key \"oops\""),
            (json!({"inputs": {"o": {"type": "object", "required": ["x"], "properties": {}}}}), "not in properties"),
            (json!({"presets": {"p": {"nope": 1}}}), "not in meta.inputs"),
            (json!({"inputs": {"w": {"type": "number"}}, "presets": {"p": {"w": "x"}}}), "input \"w\""),
        ];
        for (raw, want) in cases {
            let err = parse(raw.clone()).unwrap_err();
            assert!(err.contains(want), "{raw}: expected {want:?} in {err:?}");
        }
    }

    #[test]
    fn values_validate_and_normalize() {
        let m = parse(json!({
            "inputs": {
                "w": { "type": "number", "minimum": 1, "default": 4 },
                "v": { "type": "vector3", "default": [0, 0, 0] },
                "c": { "type": "color", "default": "#4682b4" },
            },
        }))
        .unwrap();
        assert!(m.inputs["w"].accept("w", &json!(0)).is_err(), "below minimum");
        assert_eq!(
            m.inputs["v"].accept("v", &json!({"x": 1.0, "y": 2.0, "z": 3.0})).unwrap(),
            json!([1.0, 2.0, 3.0])
        );
        assert_eq!(m.inputs["c"].accept("c", &json!([1, 0, 0])).unwrap(), json!([1, 0, 0]));
        assert!(m.inputs["c"].accept("c", &json!([1, 0])).is_err());
    }
}
