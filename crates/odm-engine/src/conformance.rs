//! Conformance suite runner: drives a headless build engine over every test
//! in tests/conformance/<channel>/ and evaluates its `export const checks`.
//! Check format and semantics: tests/conformance/README.md.

use crate::scene;
use odm_build::BuildEngine;
use odm_store::Object;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    #[serde(default)]
    t: f64,
    volume: Option<[f64; 2]>,
    area: Option<[f64; 2]>,
    bounds: Option<BoundsCheck>,
    raycast: Option<RaycastCheck>,
    error: Option<String>,
    console: Option<Vec<String>>,
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
            std::fs::copy(&entry, tmp.path().join("main.js")).expect("copy test file");
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
        .get("main.js")
        .ok_or_else(|| vec!["no main.js".to_string()])?;
    let api = main.api.clone().map_err(|e| vec![format!("pragma: {e}")])?;

    let checks = odm_js::extract_export(
        &env, "main.js", &main.code, api, "checks", kernel.clone(), store.clone(),
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
        || check.raycast.is_some();

    let pass = engine.start_pass(sync, check.t);
    let result = engine.build_root(&pass);

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
    if let Some(r) = &check.raycast {
        let hit = scene::raycast(&engine.kernel, &scene.instances, r.origin, r.dir);
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
