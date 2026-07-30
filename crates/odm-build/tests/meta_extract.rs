//! Metadata extraction: `export const meta` is read by evaluating the
//! module (no build), validated, and cached by code hash. Schema-shape
//! validation itself is unit-tested in src/meta.rs; this exercises the
//! JS boundary.

use odm_build::BuildEngine;
use odm_js::JsEnv;
use odm_kernel::Kernel;
use odm_store::Store;
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, OnceLock};

fn env() -> Arc<JsEnv> {
    static ENV: OnceLock<Arc<JsEnv>> = OnceLock::new();
    ENV.get_or_init(|| Arc::new(JsEnv::new().unwrap())).clone()
}

fn engine(project: &Path) -> Arc<BuildEngine> {
    let store = Store::new();
    let kernel = Kernel::new(store.clone());
    BuildEngine::new(store, kernel, env(), project.to_path_buf())
}

fn write(dir: &Path, path: &str, content: &str) {
    std::fs::write(dir.join(path), content).unwrap();
}

#[test]
fn meta_is_extracted_and_normalized() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "wheel.js",
        r#"//! odm unstable
//! A wheel.
//!
//! Radius and offset are knobs.
export const meta = {
  inputs: {
    radius: { type: 'number', default: 8, minimum: 1, description: 'outer radius' },
    t: { type: 'number', cascade: true, default: 0, minimum: 0, maximum: 2 },
    offset: { type: 'vector3', default: new THREE.Vector3(1, 2, 3) },
  },
  presets: {
    big: { radius: 20 },
  },
};
export default function build(ctx) { return odm.cylinder(1, 1); }
"#,
    );
    let e = engine(dir.path());
    let sync = e.sync().unwrap();
    let source = &sync.snapshot.sources["wheel.js"];
    assert_eq!(source.description, "A wheel.\n\nRadius and offset are knobs.");

    let meta = e.meta("wheel.js", source);
    let meta = meta.as_ref().as_ref().expect("meta parses");
    assert_eq!(meta.inputs.len(), 3);
    assert!(meta.inputs["t"].cascade);
    // A THREE.Vector3 default is normalized to the canonical wire form.
    assert_eq!(meta.inputs["offset"].default, Some(json!([1, 2, 3])));
    assert_eq!(meta.presets["big"]["radius"], json!(20));

    // Cached by code hash: same source → the same Arc.
    let again = e.meta("wheel.js", source);
    assert!(Arc::ptr_eq(&again, &e.meta("wheel.js", source)));
}

#[test]
fn missing_meta_is_empty_and_errors_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "plain.js", "//! odm unstable\nexport default () => odm.box(1);\n");
    write(dir.path(), "broken.js", "//! odm unstable\nthrow new Error('top-level boom');\n");
    write(
        dir.path(),
        "bad-meta.js",
        "//! odm unstable\nexport const meta = { inputs: { w: { typ: 'number' } } };\n\
         export default () => null;\n",
    );

    let e = engine(dir.path());
    let sync = e.sync().unwrap();

    let plain = e.meta("plain.js", &sync.snapshot.sources["plain.js"]);
    let plain = plain.as_ref().as_ref().expect("no meta export is fine");
    assert!(plain.inputs.is_empty() && plain.presets.is_empty());

    let broken = e.meta("broken.js", &sync.snapshot.sources["broken.js"]);
    let err = broken.as_ref().as_ref().unwrap_err();
    assert!(err.contains("top-level boom"), "{err}");

    let bad = e.meta("bad-meta.js", &sync.snapshot.sources["bad-meta.js"]);
    let err = bad.as_ref().as_ref().unwrap_err();
    assert!(err.contains("unknown key \"typ\""), "{err}");
}
