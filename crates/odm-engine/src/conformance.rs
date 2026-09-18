//! Conformance suite runner: drives a headless build engine over every test
//! in tests/conformance/<channel>/ and evaluates its `export const checks`.
//! Check format and semantics: tests/conformance/README.md.

use crate::scene;
use odm_build::{BuildEngine, View};
use odm_store::Object;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    /// Shorthand for `set: { t: ... }`.
    #[serde(default)]
    t: f64,
    /// View-level input values for this check (all become view args or
    /// cascade values, split like a CLI request's `inputs`).
    #[serde(default)]
    set: serde_json::Map<String, Value>,
    volume: Option<[f64; 2]>,
    area: Option<[f64; 2]>,
    bounds: Option<BoundsCheck>,
    raycast: Option<RaycastCheck>,
    error: Option<String>,
    console: Option<Vec<String>>,
    /// Address one node (name or index path); `volume`/`bounds` then measure
    /// that node's subtree instead of the whole scene, and `color`/`opacity`
    /// assert what was *authored* on it.
    node: Option<String>,
    /// Authored color of the addressed node: `'#rrggbb'`, `[r,g,b,a]` (sRGB
    /// 0..1), or `null` for "nothing set here". Needs `node`.
    #[serde(default, deserialize_with = "present")]
    color: Option<Value>,
    /// Authored opacity of the addressed node, or `null`. Needs `node`.
    #[serde(default, deserialize_with = "present")]
    opacity: Option<Value>,
    /// The multiset of *effective* per-instance colors the renderer will
    /// draw: `[color, alpha]` per instance, order-insensitive. `color` is
    /// `'#rrggbb'` or `null` (the uncolored default); `alpha` is the color's
    /// own alpha times the ancestors' opacity product.
    flat: Option<Vec<[Value; 2]>>,
    /// Distinct mesh hashes reachable from the scene root: "shared geometry
    /// is interned once".
    meshes: Option<usize>,
}

/// Distinguishes an explicit `null` from an absent key: serde only calls a
/// `deserialize_with` when the key is present, so absent stays `None`.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundsCheck {
    min: [f64; 3],
    max: [f64; 3],
    eps: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RaycastCheck {
    origin: [f64; 3],
    dir: [f64; 3],
    #[serde(default)]
    miss: bool,
    distance: Option<[f64; 2]>,
    normal: Option<[f64; 3]>,
    name: Option<String>,
    #[serde(default = "default_ray_eps")]
    eps: f64,
}

fn default_ray_eps() -> f64 {
    1e-6
}

fn suite_dir(channel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance").join(channel)
}

#[test]
fn unstable_suite() {
    let dir = suite_dir("unstable");
    let mut entries: Vec<PathBuf> =
        std::fs::read_dir(&dir).expect("tests/conformance/unstable exists").flatten().map(|e| e.path()).collect();
    entries.sort();
    assert!(!entries.is_empty(), "empty conformance suite at {}", dir.display());

    let mut failures = Vec::new();
    let mut ran = 0;
    for entry in entries {
        let name = entry.file_name().unwrap().to_string_lossy().into_owned();
        let project: PathBuf;
        let _hold; // keeps a single-file test's temp project alive
        if entry.is_dir() {
            project = entry.clone();
        } else if name.ends_with(".js") {
            let tmp = tempfile::tempdir().expect("tempdir");
            std::fs::copy(&entry, tmp.path().join("root.js")).expect("copy test file");
            project = tmp.path().to_path_buf();
            _hold = tmp;
        } else {
            continue; // README.md etc.
        }
        ran += 1;
        if let Err(mut errs) = run_test(&project) {
            failures.extend(errs.drain(..).map(|e| format!("{name}: {e}")));
        }
    }
    assert!(ran > 0, "no conformance tests found in {}", dir.display());
    assert!(
        failures.is_empty(),
        "{} conformance failure(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// Every name the JS API exposes must appear in the suite. Read off the
/// *live* surface rather than a hardcoded list, so a newly added function
/// fails this test until it has a test of its own.
#[test]
fn every_api_name_is_exercised() {
    // The transform/color/name methods live on a mixin base class, so the
    // walk has to go up the prototype chain, not just read own properties.
    const PROBE: &str = "//! ODM API unstable\n\
        const proto = (c) => {\n\
          const out = [];\n\
          for (let p = c.prototype; p && p !== Object.prototype; p = Object.getPrototypeOf(p)) {\n\
            out.push(...Object.getOwnPropertyNames(p));\n\
          }\n\
          return out.filter((n) => n !== 'constructor' && !n.startsWith('_'));\n\
        };\n\
        export const names = [...new Set([\n\
          ...Object.keys(odm),\n\
          ...proto(odm.Solid), ...proto(odm.Group), ...proto(odm.Instance),\n\
        ])];\n\
        export default () => odm.box(1);\n";

    // Exercised by construction or `instanceof`, never called by name.
    const ALLOWED: &[&str] = &["Solid", "Group", "Instance", "children"];

    let env = crate::state::tests::env();
    let store = odm_store::Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let names = odm_js::extract_export(
        &env,
        "root.js",
        PROBE,
        odm_build::ApiVersion::Unstable,
        "names",
        kernel,
        store,
        odm_js::EXTRACT_TIMEOUT,
    )
    .expect("read the API surface")
    .expect("the probe exports `names`");
    let names: Vec<String> = serde_json::from_value(names).expect("names is a string list");
    assert!(names.len() > 15, "only {} API names found: {names:?}", names.len());

    let suite = concat_suite(&suite_dir("unstable"));
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !ALLOWED.contains(&n.as_str()))
        .filter(|n| {
            let cap = format!("{}{}", n[..1].to_uppercase(), &n[1..]);
            !suite.contains(&format!(".{n}("))
                && !suite.contains(&format!("odm.{n}("))
                && !suite.contains(&format!("new odm.{cap}("))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "no conformance test calls {missing:?} — every API name needs one \
         (add it to a file in tests/conformance/unstable/)"
    );
}

/// Every `.js` under the suite directory, concatenated.
fn concat_suite(dir: &Path) -> String {
    let mut out = String::new();
    for entry in std::fs::read_dir(dir).expect("suite dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.push_str(&concat_suite(&path));
        } else if path.extension().is_some_and(|e| e == "js") {
            out.push_str(&std::fs::read_to_string(&path).expect("read test file"));
        }
    }
    out
}

/// All check failures for one test project (Err), or all-good (Ok).
fn run_test(project: &Path) -> Result<(), Vec<String>> {
    let env = crate::state::tests::env();
    let store = odm_store::Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let engine = BuildEngine::new(store.clone(), kernel.clone(), env.clone(), project.into());

    let sync = engine.sync().map_err(|e| vec![format!("scan: {e}")])?;
    let main = sync
        .snapshot
        .sources
        .get("root.js")
        .ok_or_else(|| vec!["no root.js".to_string()])?;
    let api = main.api.clone().map_err(|e| vec![format!("pragma: {e}")])?;

    let checks = odm_js::extract_export(
        &env,
        "root.js",
        &main.code,
        api,
        "checks",
        kernel.clone(),
        store.clone(),
        odm_js::EXTRACT_TIMEOUT,
    )
    .map_err(|e| vec![format!("reading checks: {e}")])?
    .ok_or_else(|| vec!["conformance test must `export const checks = [...]`".to_string()])?;
    let checks: Vec<Check> = serde_json::from_value(checks)
        .map_err(|e| vec![format!("bad checks format: {e}")])?;
    if checks.is_empty() {
        return Err(vec!["checks is empty".to_string()]);
    }

    let mut errs = Vec::new();
    for (i, check) in checks.iter().enumerate() {
        if let Err(e) = run_check(&engine, &sync, check) {
            errs.push(format!("check[{i}] (t={}): {e}", check.t));
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn run_check(
    engine: &std::sync::Arc<BuildEngine>,
    sync: &odm_build::SyncResult,
    check: &Check,
) -> Result<(), String> {
    let geometric = check.volume.is_some()
        || check.area.is_some()
        || check.bounds.is_some()
        || check.raycast.is_some()
        || check.node.is_some()
        || check.flat.is_some()
        || check.meshes.is_some();

    // Split `set` like the command layer: declared plain inputs become view
    // args, everything else view-level cascade values.
    let mut view = View::of("root.js");
    view.cascade.insert("t".into(), serde_json::json!(check.t));
    let meta = engine.meta("root.js", &sync.snapshot.sources["root.js"]);
    for (name, value) in &check.set {
        match meta.as_ref().as_ref().ok().and_then(|m| m.inputs.get(name)) {
            Some(input) if !input.cascade => view.args.insert(name.clone(), value.clone()),
            _ => view.cascade.insert(name.clone(), value.clone()),
        };
    }
    let pass = engine.start_pass(sync, view);
    let result = engine.build_view(&pass);

    if let Some(want) = &check.error {
        if geometric || check.console.is_some() {
            return Err("an `error` check cannot also assert geometry/console".into());
        }
        return match &result {
            Err(f) if f.message.contains(want) => Ok(()),
            Err(f) => Err(format!("build failed, but {:?} not in: {}", want, f.message)),
            Ok(_) => Err(format!("build succeeded; expected an error containing {want:?}")),
        };
    }
    let result = result.map_err(|f| format!("build failed: {}", f.message))?;

    if let Some(subs) = &check.console {
        for sub in subs {
            if !result.logs.iter().any(|(_, l)| l.message.contains(sub)) {
                let got: Vec<&str> =
                    result.logs.iter().map(|(_, l)| l.message.as_str()).collect();
                return Err(format!("console missing {sub:?}; logs: {got:?}"));
            }
        }
    }
    if !geometric {
        return Ok(());
    }

    let root = match engine.store.get(result.root).as_deref() {
        Some(Object::Node(n)) => n.clone(),
        _ => return Err("root missing from store".into()),
    };
    let scene = odm_render::flatten_node(&engine.store, &root)
        .map_err(|e| format!("flatten: {e}"))?;

    if let Some(n) = check.meshes {
        let got = scene.meshes.len();
        if got != n {
            return Err(format!("meshes: got {got} distinct mesh hashes, want {n}"));
        }
    }
    if let Some(want) = &check.flat {
        check_flat(&scene, want)?;
    }

    // `node` rescopes the measurements to one addressed node's subtree and
    // opens the authored-attribute checks.
    let located = match &check.node {
        Some(addr) => Some(
            scene::locate(&engine.store, &root, addr).map_err(|e| format!("node: {e}"))?,
        ),
        None => {
            if check.color.is_some() || check.opacity.is_some() {
                return Err("`color`/`opacity` assert an authored attribute: add `node`".into());
            }
            None
        }
    };

    if let Some((id, node, parent)) = &located {
        if check.area.is_some() {
            return Err("`area` is scene-wide and ignores transforms; drop `node`".into());
        }
        if let Some(want) = &check.color {
            let got = node.color.map(|c| [c.r as f64, c.g as f64, c.b as f64, c.a as f64]);
            match (want_color(want)?, got) {
                (None, None) => {}
                (Some(w), Some(g)) => {
                    for k in 0..4 {
                        near(g[k], w[k], 1e-6, &format!("color[{k}]"))?;
                    }
                }
                (w, g) => return Err(format!("color: got {g:?}, want {w:?}")),
            }
        }
        if let Some(want) = &check.opacity {
            let got = node.opacity.map(|o| o as f64);
            match (want.as_f64(), got) {
                (None, None) if want.is_null() => {}
                (Some(w), Some(g)) => near(g, w, 1e-6, "opacity")?,
                (w, g) => return Err(format!("opacity: got {g:?}, want {w:?}")),
            }
        }
        if check.volume.is_some() || check.bounds.is_some() {
            let fields = scene::Fields {
                volume: check.volume.is_some(),
                bounds: check.bounds.is_some(),
                ..scene::Fields::default()
            };
            let mut inspector = scene::Inspector::new(&engine.store, &engine.kernel, fields);
            let entry = inspector
                .inspect(node, id, parent, 0)
                .ok_or_else(|| format!("node {id:?} is not fully in the store"))?;
            if let Some([want, eps]) = check.volume {
                let got = entry["volume"].as_f64().ok_or("node volume unavailable")?;
                near(got, want, eps, "volume")?;
            }
            if let Some(b) = &check.bounds {
                let got = &entry["bounds"];
                if got.is_null() {
                    return Err("bounds: the addressed subtree is empty".into());
                }
                for k in 0..3 {
                    let (min, max) = (&got["min"][k], &got["max"][k]);
                    near(min.as_f64().unwrap_or(f64::NAN), b.min[k], b.eps, &format!("bounds.min[{k}]"))?;
                    near(max.as_f64().unwrap_or(f64::NAN), b.max[k], b.eps, &format!("bounds.max[{k}]"))?;
                }
            }
        }
    } else {
        if let Some([want, eps]) = check.volume {
            let mut got = 0.0;
            for inst in &scene.instances {
                let v = engine.kernel.volume(inst.mesh).map_err(|e| format!("volume: {e}"))?;
                got += v * det3(&inst.world).abs();
            }
            near(got, want, eps, "volume")?;
        }
        if let Some([want, eps]) = check.area {
            let mut got = 0.0;
            for inst in &scene.instances {
                got += engine.kernel.surface_area(inst.mesh).map_err(|e| format!("area: {e}"))?;
            }
            near(got, want, eps, "area")?;
        }
        if let Some(b) = &check.bounds {
            let Some((min, max)) = scene.bounds else {
                return Err("bounds: scene is empty".into());
            };
            for k in 0..3 {
                near(min[k], b.min[k], b.eps, &format!("bounds.min[{k}]"))?;
                near(max[k], b.max[k], b.eps, &format!("bounds.max[{k}]"))?;
            }
        }
    }
    if let Some(r) = &check.raycast {
        let hit = scene::raycast(&engine.kernel, &scene.instances, r.origin, r.dir, 1e9);
        match (&hit, r.miss) {
            (Some(h), false) => {
                if let Some([want, eps]) = r.distance {
                    let got = h["distance"].as_f64().unwrap_or(f64::NAN);
                    near(got, want, eps, "raycast.distance")?;
                }
                if let Some(want) = r.normal {
                    let got = &h["normal"];
                    for k in 0..3 {
                        let g = got[k].as_f64().unwrap_or(f64::NAN);
                        near(g, want[k], r.eps, &format!("raycast.normal[{k}]"))?;
                    }
                }
                if let Some(want) = &r.name {
                    let got = h["name"].as_str().unwrap_or("");
                    if got != want {
                        return Err(format!("raycast.name: got {got:?}, want {want:?}"));
                    }
                }
            }
            (None, true) => {}
            (Some(h), true) => return Err(format!("raycast: expected a miss, hit {h}")),
            (None, false) => return Err("raycast: expected a hit, missed".into()),
        }
    }
    Ok(())
}

/// sRGB→linear, the copy of `odm_render`'s private crossing that lets a check
/// author sRGB and compare against what the flattener produced.
fn to_linear(c: [f64; 4]) -> [f64; 4] {
    fn ch(c: f64) -> f64 {
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    [ch(c[0]), ch(c[1]), ch(c[2]), c[3]]
}

/// A check's color literal: `'#rgb'`/`'#rrggbb'`/`'#rrggbbaa'`, `[r,g,b]` or
/// `[r,g,b,a]` in sRGB 0..1 — the formats `.color()` itself takes — or `null`
/// for "no color here".
fn want_color(v: &Value) -> Result<Option<[f64; 4]>, String> {
    if v.is_null() {
        return Ok(None);
    }
    if let Some(hex) = v.as_str() {
        let h = hex.strip_prefix('#').ok_or_else(|| format!("bad color {hex:?}"))?;
        let digits: Vec<f64> = match h.len() {
            3 => h.chars().map(|c| c.to_digit(16).map(|d| (d * 17) as f64 / 255.0)).collect::<Option<_>>(),
            6 | 8 => (0..h.len() / 2)
                .map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok().map(|b| b as f64 / 255.0))
                .collect::<Option<_>>(),
            _ => None,
        }
        .ok_or_else(|| format!("bad color {hex:?}"))?;
        return Ok(Some([digits[0], digits[1], digits[2], *digits.get(3).unwrap_or(&1.0)]));
    }
    let a = v.as_array().ok_or_else(|| format!("bad color {v}"))?;
    let nums: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect();
    match nums.len() {
        3 => Ok(Some([nums[0], nums[1], nums[2], 1.0])),
        4 => Ok(Some([nums[0], nums[1], nums[2], nums[3]])),
        _ => Err(format!("bad color {v}")),
    }
}

/// The `flat` check: the multiset of effective instance colors, compared
/// order-insensitively against `[color, alpha]` pairs.
fn check_flat(scene: &odm_render::RenderScene, want: &[[Value; 2]]) -> Result<(), String> {
    let mut wanted = Vec::new();
    for [color, alpha] in want {
        let rgb = match want_color(color)? {
            Some(c) => to_linear(c),
            // No color anywhere up the tree: the flattener's own default,
            // which is already linear.
            None => odm_render::DEFAULT_COLOR.map(f64::from),
        };
        let a = alpha.as_f64().ok_or_else(|| format!("flat alpha must be a number, got {alpha}"))?;
        wanted.push([rgb[0], rgb[1], rgb[2], a]);
    }
    let mut got: Vec<[f64; 4]> =
        scene.instances.iter().map(|i| i.color.map(f64::from)).collect();
    if got.len() != wanted.len() {
        return Err(format!("flat: {} instances, want {}", got.len(), wanted.len()));
    }
    let key = |c: &[f64; 4]| c.map(|x| (x * 1e6).round() as i64);
    got.sort_by_key(key);
    wanted.sort_by_key(key);
    for (g, w) in got.iter().zip(&wanted) {
        if key(g) != key(w) {
            return Err(format!("flat: got {got:?}, want {wanted:?}"));
        }
    }
    Ok(())
}

fn near(got: f64, want: f64, eps: f64, what: &str) -> Result<(), String> {
    if (got - want).abs() <= eps && got.is_finite() {
        Ok(())
    } else {
        Err(format!("{what}: got {got}, want {want} ± {eps}"))
    }
}

/// Determinant of the linear 3×3 block of a column-major 4×4.
fn det3(m: &[f64; 16]) -> f64 {
    m[0] * (m[5] * m[10] - m[6] * m[9]) - m[4] * (m[1] * m[10] - m[2] * m[9])
        + m[8] * (m[1] * m[6] - m[2] * m[5])
}
