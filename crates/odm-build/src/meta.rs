//! Part metadata: `export const meta = { inputs: {...}, presets: {...} }`.
//!
//! `meta.inputs` is ONE map, name → entry; an entry is a profiled JSON
//! Schema (`type`, `enum`, `default`, `description`, `minimum`/`maximum`,
//! `items`, `properties`/`required`, `additionalProperties`, `variants`/
//! `tag`) plus the ODM key `cascade` (top level only). The grammar is
//! recursive: a nested schema at any depth is the top-level grammar minus
//! `cascade`. Unknown keys are rejected by our own allowlist walk — which is
//! also where ODM keys are recognized, so a typo'd key errors instead of
//! silently changing an input's kind. ODM keys are stripped and extension
//! types desugared before value validation, which is delegated to the
//! `jsonschema` crate. `default` has real semantics at every depth: an
//! absent object property with a declared default is filled in at
//! normalization (an array `items.default` is different in kind — absent
//! elements don't exist — it is the panel's new-element template).
//! Metadata is read by evaluating the module (no build); callers cache the
//! result by code hash (see `BuildEngine::meta`).

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// ODM's extension types: `type` values beyond JSON Schema's own. Each has
/// exactly one wire form (vectors `[x, y, z]`, matrix4 = 16 numbers
/// column-major, color = hex string or `[r, g, b]`): the framework turns
/// THREE instances into it before any boundary, and nothing else is
/// accepted. `ctx.input` hydrates the wire form into real THREE instances,
/// declaration-driven, at any depth. `solid` is top-level-only: an opaque
/// handle with no authorable value.
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
}

/// The extension type a schema node's `type` names, if any.
fn ext_of(entry: &Map<String, Value>) -> Option<ExtType> {
    entry.get("type").and_then(|t| t.as_str()).and_then(ExtType::parse)
}

/// A union's tag property name: `tag` if declared, else `kind`.
pub fn tag_name(entry: &Map<String, Value>) -> &str {
    entry.get("tag").and_then(|t| t.as_str()).unwrap_or("kind")
}

/// One declared input.
#[derive(Debug)]
pub struct Input {
    /// `cascade: true`: resolved up the invoke chain — nearest explicitly
    /// provided value, view outermost; `default` mandatory. Otherwise the value
    /// comes from the immediate caller's args, falling back to `default`
    /// (no default = required).
    pub cascade: bool,
    /// Top-level extension type, if the input's own `type` names one.
    pub extension: Option<ExtType>,
    /// The entry as authored, with every `default` in the tree normalized
    /// (nested defaults filled). The panel's and the report's schema.
    pub authored: Map<String, Value>,
    /// Normalized top-level default, if declared.
    pub default: Option<Value>,
    validator: jsonschema::Validator,
}

impl Input {
    /// Normalize a value for this input — absent defaulted object
    /// properties filled, at any depth — and validate it against the schema.
    pub fn accept(&self, name: &str, value: &Value) -> Result<Value, String> {
        let v = normalize(&format!("input {name:?}"), &self.authored, value)?;
        match self.validator.validate(&v) {
            Ok(()) => Ok(v),
            Err(e) => {
                let at = match e.instance_path().as_str() {
                    "" => String::new(),
                    p => format!(", at {p}"),
                };
                let ty = self
                    .authored
                    .get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| format!(" (declared type: {t})"))
                    .unwrap_or_default();
                Err(format!("input {name:?}: {e}{at}{ty}"))
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

/// A part's parsed `export const meta`. Absent export = empty meta.
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
                let hint = match key.as_str() {
                    "description" => "; the file's description is its leading `//!` comment block",
                    _ => "",
                };
                return Err(format!("unknown meta key {key:?}; allowed: inputs, presets{hint}"));
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
    let at = format!("{name:?}");
    let extension = check_schema(&at, entry, true)?;

    let cascade = match entry.get("cascade") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(format!("{name:?}: cascade must be true or false")),
    };
    if extension == Some(ExtType::Solid) && entry.contains_key("default") {
        return Err(format!("{name:?}: solid inputs cannot have a default"));
    }

    // Normalize and validate every `default` in the tree, depth-first, into
    // the authored copy — the schema everyone else (report, panel, decls)
    // sees carries canonical defaults.
    let mut authored = entry.clone();
    process_defaults(&at, &mut authored)?;

    let mut for_validator = authored.clone();
    for_validator.remove("cascade");
    let validator = jsonschema::validator_for(&desugar(&for_validator))
        .map_err(|e| format!("{name:?}: schema does not compile: {e}"))?;

    let default = authored.get("default").cloned();
    if default.is_none() && cascade {
        return Err(format!("{name:?}: a cascade input must declare a default"));
    }
    Ok(Input { cascade, extension, authored, default, validator })
}

/// The profile allowlist walk. Returns the extension type if `type` names
/// one. `top` additionally admits the ODM key `cascade`, which has no
/// meaning in nested schemas (it names a resolution channel, not a shape).
fn check_schema(at: &str, entry: &Map<String, Value>, top: bool) -> Result<Option<ExtType>, String> {
    const NESTED_KEYS: &[&str] = &[
        "type",
        "enum",
        "default",
        "description",
        "minimum",
        "maximum",
        "items",
        "properties",
        "required",
        "additionalProperties",
        "variants",
        "tag",
    ];
    for key in entry.keys() {
        let ok = NESTED_KEYS.contains(&key.as_str()) || (top && key == "cascade");
        if !ok {
            return Err(format!(
                "{at}: unknown key {key:?}; allowed: {}{}",
                NESTED_KEYS.join(", "),
                if top { ", cascade" } else { "" }
            ));
        }
    }
    if let Some(d) = entry.get("description")
        && !d.is_string()
    {
        return Err(format!("{at}: description must be a string"));
    }

    // A tagged union: `variants` is its own kind of node — bodies are
    // object schemas selected by the tag property.
    if let Some(vars) = entry.get("variants") {
        for key in ["type", "enum", "minimum", "maximum", "items", "properties", "required", "additionalProperties"] {
            if entry.contains_key(key) {
                return Err(format!("{at}: {key:?} cannot be combined with \"variants\""));
            }
        }
        let tag = match entry.get("tag") {
            None => "kind",
            Some(Value::String(s)) if !s.is_empty() => s.as_str(),
            Some(_) => return Err(format!("{at}: tag must be a non-empty string")),
        };
        let Some(vars) = vars.as_object().filter(|v| !v.is_empty()) else {
            return Err(format!("{at}: variants must be a non-empty object of variant bodies"));
        };
        for (vname, body) in vars {
            let vat = format!("{at}.{vname}");
            let Some(body) = body.as_object() else {
                return Err(format!("{vat}: a variant body must be an object schema"));
            };
            for key in body.keys() {
                if !["properties", "required", "description"].contains(&key.as_str()) {
                    return Err(format!(
                        "{vat}: unknown variant body key {key:?}; allowed: properties, required, description \
                         (a variant body is an object schema — the value is internally tagged)"
                    ));
                }
            }
            if body.get("properties").is_some_and(|p| {
                p.as_object().is_some_and(|p| p.contains_key(tag))
            }) {
                return Err(format!(
                    "{vat}: property {tag:?} collides with the union's tag property"
                ));
            }
            // Reuse the object-schema checks for properties/required.
            let mut as_object = body.clone();
            as_object.insert("type".into(), Value::String("object".into()));
            check_schema(&vat, &as_object, false)?;
        }
        return Ok(None);
    }
    if entry.contains_key("tag") {
        return Err(format!("{at}: \"tag\" needs \"variants\""));
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
            Some(ExtType::Solid) if !top => {
                return Err(format!(
                    "{at}: type \"solid\" is only supported at the top level of an input \
                     (a solid is an opaque handle with no authorable value)"
                ));
            }
            Some(e) => Some(e),
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
        for key in ["enum", "minimum", "maximum", "items", "properties", "required", "additionalProperties"] {
            if entry.contains_key(key) {
                return Err(format!(
                    "{at}: {key:?} cannot be combined with the extension type {:?}",
                    extension.unwrap().name()
                ));
            }
        }
        return Ok(extension);
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
    // A string-keyed map: `additionalProperties: <schema>` — a map, not a
    // record, so it excludes `properties`/`required`.
    if let Some(ap) = entry.get("additionalProperties") {
        if ty != Some("object") {
            return Err(format!("{at}: additionalProperties needs type \"object\""));
        }
        for key in ["properties", "required"] {
            if entry.contains_key(key) {
                return Err(format!(
                    "{at}: {key:?} cannot be combined with additionalProperties \
                     (a map has no fixed keys)"
                ));
            }
        }
        let Some(ap) = ap.as_object() else {
            return Err(format!("{at}: additionalProperties must be a schema object"));
        };
        check_schema(&format!("{at}.additionalProperties"), ap, false)?;
    }
    Ok(None)
}

/// Depth-first over the schema tree: normalize and validate every `default`,
/// writing the canonical (deep-filled) form back. Children first, so a
/// parent's default is filled with already-normalized child defaults.
fn process_defaults(at: &str, entry: &mut Map<String, Value>) -> Result<(), String> {
    if let Some(items) = entry.get_mut("items").and_then(|v| v.as_object_mut()) {
        process_defaults(&format!("{at}.items"), items)?;
    }
    if let Some(props) = entry.get_mut("properties").and_then(|v| v.as_object_mut()) {
        for (pname, pschema) in props.iter_mut() {
            let pat = format!("{at}.{pname}");
            let pschema = pschema.as_object_mut().expect("checked by check_schema");
            process_defaults(&pat, pschema)?;
        }
    }
    if let Some(ap) = entry.get_mut("additionalProperties").and_then(|v| v.as_object_mut()) {
        process_defaults(&format!("{at}.additionalProperties"), ap)?;
    }
    if let Some(vars) = entry.get_mut("variants").and_then(|v| v.as_object_mut()) {
        for (vname, body) in vars.iter_mut() {
            let body = body.as_object_mut().expect("checked by check_schema");
            if let Some(props) = body.get_mut("properties").and_then(|v| v.as_object_mut()) {
                for (pname, pschema) in props.iter_mut() {
                    let pat = format!("{at}.{vname}.{pname}");
                    let pschema = pschema.as_object_mut().expect("checked by check_schema");
                    process_defaults(&pat, pschema)?;
                }
            }
        }
    }
    if let Some(raw) = entry.get("default").cloned() {
        if ext_of(entry) == Some(ExtType::Solid) {
            return Err(format!("{at}: solid inputs cannot have a default"));
        }
        let schema = entry.clone();
        let d = normalize(at, &schema, &raw).map_err(|e| format!("bad default for {e}"))?;
        let validator = jsonschema::validator_for(&desugar(&schema))
            .map_err(|e| format!("{at}: schema does not compile: {e}"))?;
        if let Err(e) = validator.validate(&d) {
            return Err(format!("bad default for {at}: {e}"));
        }
        entry.insert("default".into(), d);
    }
    Ok(())
}

/// The standard-JSON-Schema equivalent of a profiled schema node, built
/// recursively for the `jsonschema` validator: extension types desugar to
/// their wire shapes, `variants` to a tag enum plus if/then branches (so a
/// valid tag activates exactly its own branch's errors), and only
/// validation-relevant keys are carried.
fn desugar(entry: &Map<String, Value>) -> Value {
    if let Some(vars) = entry.get("variants").and_then(|v| v.as_object()) {
        let tag = tag_name(entry);
        let names: Vec<Value> = vars.keys().cloned().map(Value::String).collect();
        // One if/then per variant, keyed on the tag value, so a valid tag
        // activates exactly its own branch's errors.
        let branches: Vec<Value> = vars
            .iter()
            .map(|(vname, body)| {
                let body = body.as_object().expect("checked by check_schema");
                let mut then = Map::new();
                then.insert("type".into(), json!("object"));
                if let Some(props) = body.get("properties").and_then(|p| p.as_object()) {
                    let dp: Map<String, Value> = props
                        .iter()
                        .map(|(k, s)| (k.clone(), desugar(s.as_object().expect("schema object"))))
                        .collect();
                    then.insert("properties".into(), Value::Object(dp));
                }
                if let Some(req) = body.get("required") {
                    then.insert("required".into(), req.clone());
                }
                json!({
                    "if": { "properties": { tag: { "const": vname } }, "required": [tag] },
                    "then": then,
                })
            })
            .collect();
        return json!({
            "type": "object",
            "properties": { tag: { "enum": names } },
            "required": [tag],
            "allOf": branches,
        });
    }
    if let Some(ext) = ext_of(entry) {
        return ext.desugar();
    }
    let mut out = Map::new();
    for key in ["type", "enum", "minimum", "maximum", "required"] {
        if let Some(v) = entry.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    if let Some(items) = entry.get("items").and_then(|v| v.as_object()) {
        out.insert("items".into(), desugar(items));
    }
    if let Some(props) = entry.get("properties").and_then(|v| v.as_object()) {
        let dp: Map<String, Value> = props
            .iter()
            .map(|(k, s)| (k.clone(), desugar(s.as_object().expect("schema object"))))
            .collect();
        out.insert("properties".into(), Value::Object(dp));
    }
    if let Some(ap) = entry.get("additionalProperties").and_then(|v| v.as_object()) {
        out.insert("additionalProperties".into(), desugar(ap));
    }
    Value::Object(out)
}

/// Deep normalize a value against a schema node: THREE-instance forms →
/// absent object properties filled from their declared (already-normalized)
/// defaults, union branches selected by tag. Anything off-schema (an
/// extension type in any spelling but its wire form included) passes
/// through untouched for
/// validation to report — except a bad or missing union tag, which errors
/// here with a message naming the tag (the desugared schema's own error
/// names everything but).
fn normalize(at: &str, entry: &Map<String, Value>, v: &Value) -> Result<Value, String> {
    if let Some(vars) = entry.get("variants").and_then(|x| x.as_object()) {
        let tag = tag_name(entry);
        let names = vars.keys().cloned().collect::<Vec<_>>().join(", ");
        let Some(obj) = v.as_object() else {
            return Err(format!(
                "{at}: a union value must be an object carrying the tag property {tag:?} \
                 (variants: {names})"
            ));
        };
        let Some(kind) = obj.get(tag).and_then(|k| k.as_str()) else {
            return Err(format!(
                "{at}: missing union tag — set {tag:?} to one of: {names}"
            ));
        };
        let Some(body) = vars.get(kind).and_then(|b| b.as_object()) else {
            return Err(format!(
                "{at}: unknown variant {kind:?} for tag {tag:?}; variants: {names}"
            ));
        };
        return normalize_object(at, body.get("properties").and_then(|p| p.as_object()), obj);
    }
    match entry.get("type").and_then(|t| t.as_str()) {
        Some("object") => {
            let Some(obj) = v.as_object() else { return Ok(v.clone()) };
            if let Some(props) = entry.get("properties").and_then(|p| p.as_object()) {
                normalize_object(at, Some(props), obj)
            } else if let Some(ap) = entry.get("additionalProperties").and_then(|p| p.as_object())
            {
                let mut out = Map::new();
                for (k, val) in obj {
                    out.insert(k.clone(), normalize(&format!("{at}.{k}"), ap, val)?);
                }
                Ok(Value::Object(out))
            } else {
                Ok(v.clone())
            }
        }
        Some("array") => {
            let (Some(items), Some(arr)) =
                (entry.get("items").and_then(|i| i.as_object()), v.as_array())
            else {
                return Ok(v.clone());
            };
            let mut out = Vec::with_capacity(arr.len());
            for (i, el) in arr.iter().enumerate() {
                out.push(normalize(&format!("{at}[{i}]"), items, el)?);
            }
            Ok(Value::Array(out))
        }
        _ => Ok(v.clone()),
    }
}

/// Normalize an object value against a `properties` map, filling absent
/// properties that declare a default. Keys outside `properties` (a union's
/// tag, junk for the validator) pass through untouched.
fn normalize_object(
    at: &str,
    props: Option<&Map<String, Value>>,
    obj: &Map<String, Value>,
) -> Result<Value, String> {
    let mut out = Map::new();
    for (k, val) in obj {
        let normalized = match props.and_then(|p| p.get(k)).and_then(|s| s.as_object()) {
            Some(schema) => normalize(&format!("{at}.{k}"), schema, val)?,
            None => val.clone(),
        };
        out.insert(k.clone(), normalized);
    }
    if let Some(props) = props {
        for (k, schema) in props {
            if out.contains_key(k) {
                continue;
            }
            if let Some(d) = schema.as_object().and_then(|s| s.get("default")) {
                out.insert(k.clone(), d.clone());
            }
        }
    }
    Ok(Value::Object(out))
}

/// A fresh value synthesized from a schema node: its declared `default` if
/// any, else nested defaults where declared and type-blanks elsewhere, with
/// required properties filled recursively. The panel's new-element template
/// and new-map-entry value; also how a union switch builds the incoming
/// variant's subtree.
pub fn synthesize(entry: &Map<String, Value>) -> Value {
    if let Some(d) = entry.get("default") {
        return d.clone();
    }
    if let Some(vars) = entry.get("variants").and_then(|v| v.as_object()) {
        // No default: the first declared variant.
        let Some(name) = vars.keys().next() else { return Value::Null };
        return synthesize_variant(entry, name);
    }
    if let Some(choices) = entry.get("enum").and_then(|e| e.as_array()) {
        return choices.first().cloned().unwrap_or(Value::Null);
    }
    match entry.get("type").and_then(|t| t.as_str()) {
        Some("number") | Some("integer") => {
            // Respect a declared range: 0 unless the minimum pushes it up.
            let lo = entry.get("minimum").and_then(|v| v.as_f64());
            let hi = entry.get("maximum").and_then(|v| v.as_f64());
            let v = 0f64.max(lo.unwrap_or(0.0)).min(hi.unwrap_or(f64::INFINITY));
            if v.fract() == 0.0 && v.abs() < 9e15 { json!(v as i64) } else { json!(v) }
        }
        Some("string") => json!(""),
        Some("boolean") => json!(false),
        Some("array") => json!([]),
        Some("object") => {
            let mut out = Map::new();
            if let Some(props) = entry.get("properties").and_then(|p| p.as_object()) {
                let required: Vec<&str> = entry
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                for (k, schema) in props {
                    let Some(schema) = schema.as_object() else { continue };
                    if schema.contains_key("default") || required.contains(&k.as_str()) {
                        out.insert(k.clone(), synthesize(schema));
                    }
                }
            }
            Value::Object(out)
        }
        Some("vector2") => json!([0, 0]),
        Some("vector3") => json!([0, 0, 0]),
        Some("quaternion") => json!([0, 0, 0, 1]),
        Some("matrix4") => json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]),
        Some("color") => json!("#888888"),
        _ => Value::Null,
    }
}

/// Synthesize one named variant of a union node: the tag plus that body's
/// synthesized properties (defaults and required fills).
pub fn synthesize_variant(entry: &Map<String, Value>, name: &str) -> Value {
    let tag = tag_name(entry);
    let mut out = Map::new();
    out.insert(tag.to_string(), Value::String(name.to_string()));
    if let Some(body) = entry
        .get("variants")
        .and_then(|v| v.as_object())
        .and_then(|vars| vars.get(name))
        .and_then(|b| b.as_object())
    {
        let mut as_object = body.clone();
        as_object.insert("type".into(), Value::String("object".into()));
        if let Value::Object(filled) = synthesize(&as_object) {
            out.extend(filled);
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            (json!({"description": "a box"}), "leading `//!` comment block"),
            (json!({"inputs": {"w": {"type": "number", "minimun": 3}}}), "unknown key \"minimun\""),
            (json!({"inputs": {"w": {"type": "number", "cascadee": true}}}), "unknown key \"cascadee\""),
            (json!({"inputs": {"w": {"type": "float"}}}), "unknown type \"float\""),
            (json!({"inputs": {"t": {"type": "number", "cascade": true}}}), "must declare a default"),
            (json!({"inputs": {"w": {"type": "number", "default": "wide"}}}), "bad default"),
            (json!({"inputs": {"v": {"type": "vector3", "default": [1, 2]}}}), "bad default"),
            (json!({"inputs": {"s": {"type": "solid", "default": 1}}}), "cannot have a default"),
            (json!({"inputs": {"s": {"type": "string", "minimum": 0}}}), "needs type number"),
            (json!({"inputs": {"o": {"type": "object", "properties": {"x": {"type": "number", "oops": 1}}}}}), "unknown key \"oops\""),
            (json!({"inputs": {"o": {"type": "object", "required": ["x"], "properties": {}}}}), "not in properties"),
            (json!({"presets": {"p": {"nope": 1}}}), "not in meta.inputs"),
            (json!({"inputs": {"w": {"type": "number"}}, "presets": {"p": {"w": "x"}}}), "input \"w\""),
            // The recursive grammar's own rejections.
            (json!({"inputs": {"a": {"type": "array", "items": {"type": "solid"}}}}), "only supported at the top level"),
            (json!({"inputs": {"o": {"type": "object", "properties": {"p": {"type": "number", "cascade": true}}}}}), "unknown key \"cascade\""),
            (json!({"inputs": {"o": {"type": "object", "properties": {"p": {"type": "number", "default": "x"}}}}}), "bad default for \"o\".p"),
            (json!({"inputs": {"m": {"type": "object", "additionalProperties": {"type": "number"}, "properties": {}}}}), "cannot be combined with additionalProperties"),
            (json!({"inputs": {"m": {"type": "array", "additionalProperties": {"type": "number"}}}}), "additionalProperties needs type \"object\""),
            (json!({"inputs": {"u": {"variants": {}}}}), "non-empty object"),
            (json!({"inputs": {"u": {"variants": {"a": {}}, "type": "object"}}}), "cannot be combined with \"variants\""),
            (json!({"inputs": {"u": {"variants": {"a": {"items": {}}}}}}), "unknown variant body key"),
            (json!({"inputs": {"u": {"variants": {"a": {"properties": {"kind": {"type": "string"}}}}}}}), "collides with the union's tag"),
            (json!({"inputs": {"u": {"tag": "shape"}}}), "\"tag\" needs \"variants\""),
            (json!({"inputs": {"u": {"variants": {"a": {}}, "default": {}}}}), "missing union tag"),
            (json!({"inputs": {"u": {"variants": {"a": {}}, "default": {"kind": "b"}}}}), "unknown variant"),
        ];
        for (raw, want) in cases {
            let err = parse(raw.clone()).unwrap_err();
            assert!(err.contains(want), "{raw}: expected {want:?} in {err:?}");
        }
    }

    #[test]
    fn values_validate_against_one_wire_form() {
        let m = parse(json!({
            "inputs": {
                "w": { "type": "number", "minimum": 1, "default": 4 },
                "v": { "type": "vector3", "default": [0, 0, 0] },
                "c": { "type": "color", "default": "#4682b4" },
            },
        }))
        .unwrap();
        assert!(m.inputs["w"].accept("w", &json!(0)).is_err(), "below minimum");
        assert_eq!(m.inputs["v"].accept("v", &json!([1.0, 2.0, 3.0])).unwrap(), json!([1.0, 2.0, 3.0]));
        // The {x, y, z} spelling is not a second wire form.
        let err = m.inputs["v"].accept("v", &json!({"x": 1.0, "y": 2.0, "z": 3.0})).unwrap_err();
        assert!(err.contains("declared type: vector3"), "{err}");
        assert_eq!(m.inputs["c"].accept("c", &json!([1, 0, 0])).unwrap(), json!([1, 0, 0]));
        assert!(m.inputs["c"].accept("c", &json!([1, 0])).is_err());
    }

    #[test]
    fn nested_ext_types_validate_at_depth() {
        let m = parse(json!({
            "inputs": {
                "objects": { "type": "array", "default": [], "items": { "type": "object",
                    "properties": { "position": { "type": "vector3", "default": [0, 0, 0] } } } },
            },
        }))
        .unwrap();
        let v = m.inputs["objects"].accept("objects", &json!([{ "position": [1.0, 2.0, 3.0] }])).unwrap();
        assert_eq!(v, json!([{ "position": [1.0, 2.0, 3.0] }]));
        // Validation errors name where in the value they are.
        let err = m.inputs["objects"].accept("objects", &json!([{ "position": [1, 2] }])).unwrap_err();
        assert!(err.contains("/0/position"), "path in error: {err}");
        let err = m.inputs["objects"]
            .accept("objects", &json!([{ "position": {"x": 1.0, "y": 2.0, "z": 3.0} }]))
            .unwrap_err();
        assert!(err.contains("/0/position"), "path in error: {err}");
    }

    #[test]
    fn nested_defaults_fill_absent_properties() {
        let m = parse(json!({
            "inputs": {
                "hole": { "type": "object", "default": { "r": 6 },
                    "properties": {
                        "r": { "type": "number" },
                        "depth": { "type": "number", "default": 10 },
                        "lining": { "type": "object", "properties": {
                            "thick": { "type": "number", "default": 1 } }, "default": {} },
                    } },
            },
        }))
        .unwrap();
        // The declared top default itself is deep-filled at parse.
        assert_eq!(
            m.inputs["hole"].default,
            Some(json!({ "r": 6, "depth": 10, "lining": { "thick": 1 } }))
        );
        // And so is any accepted value.
        assert_eq!(
            m.inputs["hole"].accept("hole", &json!({ "r": 2, "lining": {} })).unwrap(),
            json!({ "r": 2, "depth": 10, "lining": { "thick": 1 } })
        );
    }

    #[test]
    fn unions_select_by_tag() {
        let m = parse(json!({
            "inputs": {
                "shape": { "variants": {
                    "box": { "properties": { "size": { "type": "vector3", "default": [10, 10, 10] } } },
                    "sphere": { "properties": { "radius": { "type": "number" } }, "required": ["radius"] },
                }, "default": { "kind": "box" } },
            },
        }))
        .unwrap();
        // Union default synthesizes its branch's defaults.
        assert_eq!(m.inputs["shape"].default, Some(json!({ "kind": "box", "size": [10, 10, 10] })));
        // The selected branch validates.
        assert_eq!(
            m.inputs["shape"]
                .accept("shape", &json!({ "kind": "box", "size": [1.0, 2.0, 3.0] }))
                .unwrap(),
            json!({ "kind": "box", "size": [1.0, 2.0, 3.0] })
        );
        let err = m.inputs["shape"].accept("shape", &json!({ "kind": "cone" })).unwrap_err();
        assert!(err.contains("unknown variant \"cone\""), "{err}");
        let err = m.inputs["shape"].accept("shape", &json!({})).unwrap_err();
        assert!(err.contains("missing union tag"), "{err}");
        // A selected variant's own schema still validates: sphere requires
        // radius (no default to fill it).
        let err =
            m.inputs["shape"].accept("shape", &json!({ "kind": "sphere" })).unwrap_err();
        assert!(err.contains("radius"), "{err}");
        assert_eq!(
            m.inputs["shape"].accept("shape", &json!({ "kind": "sphere", "radius": 2 })).unwrap(),
            json!({ "kind": "sphere", "radius": 2 })
        );
    }

    #[test]
    fn maps_validate_their_values() {
        let m = parse(json!({
            "inputs": {
                "anchors": { "type": "object", "default": {},
                    "additionalProperties": { "type": "vector3" } },
            },
        }))
        .unwrap();
        assert_eq!(
            m.inputs["anchors"].accept("anchors", &json!({ "lid": [0.0, 0.0, 9.0] })).unwrap(),
            json!({ "lid": [0.0, 0.0, 9.0] })
        );
        let err = m.inputs["anchors"].accept("anchors", &json!({ "lid": 4 })).unwrap_err();
        assert!(err.contains("/lid"), "path in error: {err}");
    }

    #[test]
    fn synthesis_builds_new_element_templates() {
        let schema = |v: Value| v.as_object().unwrap().clone();
        // items.default is the template verbatim.
        assert_eq!(synthesize(&schema(json!({ "type": "number", "default": 7 }))), json!(7));
        // No default: type-blanks, with required and defaulted properties filled.
        assert_eq!(
            synthesize(&schema(json!({ "type": "object",
                "properties": {
                    "position": { "type": "vector3", "default": [0, 0, 5] },
                    "label": { "type": "string" },
                    "count": { "type": "integer", "minimum": 2 },
                },
                "required": ["count"] }))),
            json!({ "position": [0, 0, 5], "count": 2 })
        );
        // Enum: the first choice. Union: the first variant, tagged.
        assert_eq!(synthesize(&schema(json!({ "enum": ["flat", "domed"] }))), json!("flat"));
        assert_eq!(
            synthesize(&schema(json!({ "variants": {
                "box": { "properties": { "size": { "type": "vector3", "default": [1, 1, 1] } } },
                "sphere": {},
            } }))),
            json!({ "kind": "box", "size": [1, 1, 1] })
        );
    }
}
