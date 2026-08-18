//! End-to-end check of the JS half of a web export, no browser needed:
//! generate a real bundle, load it in node together with the REAL
//! runtime.js (against a mock wasm module), and drive builds through
//! `__odmWeb.runBuild` — the exact call the wasm executor makes. Covers the
//! transformer's output, the module registry, per-build fresh module scope,
//! the determinism prelude swap, and the ops glue signatures.
//!
//! Skips (with a note) when `node` is not on PATH.

use std::process::Command;

const ROOT_JS: &str = r#"//! odm unstable
//! Node-test doohickey.
import * as THREE from 'three';
export const meta = { inputs: { t: { type: 'number', minimum: 0, maximum: 1, default: 0.25, cascade: true } } };
let perBuildState = 0;
export default function build(ctx) {
  perBuildState += 1;
  const t = ctx.input('t');
  const bite = odm.sphere(1.25, { segments: 16 }).translate(1, 1, 1);
  const part = odm.box(2).subtract(bite);
  console.log('state', perBuildState, 't', t, 'rand', Math.random().toFixed(6), 'now', Date.now());
  const v = new THREE.Vector3(1, 2, 3);
  return part.color('#8888ff').translate(v.x, v.y, v.z).name('part');
}
"#;

const DRIVER: &str = r#"
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const dir = path.dirname(fileURLToPath(import.meta.url));
(0, eval)(fs.readFileSync(path.join(dir, 'bundle.js'), 'utf8'));
await import('./runtime.js');

const logs = globalThis.__mockLogs;
const run = () => JSON.parse(globalThis.__odmWeb.runBuild('root.js', 'unstable', '{}', JSON.stringify({ t: { cascade: true, type: 'number' } })));

const randBefore = Math.random;
const one = run();
if (one.error) throw new Error('build 1 failed: ' + one.error.message);
if (Math.random !== randBefore) throw new Error('Math.random not restored after build');
if (globalThis.console.log === undefined || logs.length === 0) throw new Error('no captured logs');

// Per-build fresh module scope: perBuildState resets, PRNG reseeds, Date is
// frozen — so runs are byte-identical.
const two = run();
if (JSON.stringify(one) !== JSON.stringify(two)) throw new Error('rebuild not identical');
const [l1, l2] = [logs[0], logs[1]];
if (l1 !== l2) throw new Error(`logs differ across builds:\n${l1}\n${l2}`);
if (!l1.includes('state 1')) throw new Error('module state leaked across builds: ' + l1);
if (!l1.includes('t 0.5')) throw new Error('cascade read missing: ' + l1);
if (!l1.includes('now 1700000000000')) throw new Error('Date not frozen: ' + l1);

// IR shape: a translated, colored, named node over a mock geometry handle.
const ir = one.ok;
if (ir.name !== 'part') throw new Error('bad IR: ' + JSON.stringify(ir));
if (!globalThis.__mockHandles.has(ir.geom)) throw new Error('IR does not reference a mock solid');
if (ir.matrix[12] !== 1 || ir.matrix[13] !== 2 || ir.matrix[14] !== 3) throw new Error('bad matrix');

// A doohickey the bundle does not carry: internal error envelope.
const missing = JSON.parse(globalThis.__odmWeb.runBuild('nope.js', 'unstable', '{}', '{}'));
if (!missing.error || missing.error.kind !== 'internal') throw new Error('missing-file envelope wrong');

// A throwing build: js error envelope, console still restored.
console.log('driver: all checks passed');
"#;

/// Mock of the wasm-bindgen module: enough op surface for the test
/// doohickey, handles as fake hex hashes, logs captured globally.
const MOCK_WASM: &str = r#"
export default async function init() {}
// Content-addressed like the real kernel: same args, same handle.
let n = 0;
const byKey = new Map();
const handles = (globalThis.__mockHandles = new Set());
const fake = (...key) => {
  const k = JSON.stringify(key);
  if (!byKey.has(k)) {
    const h = (++n).toString(16).padStart(64, '0');
    byKey.set(k, h);
    handles.add(h);
  }
  return byKey.get(k);
};
globalThis.__mockLogs = [];
export function op_solid_box(size, center) { return fake('box', [...size], center); }
export function op_solid_sphere(r, seg) { return fake('sphere', r, seg); }
export function op_boolean(kind, operandsJson) {
  const ops = JSON.parse(operandsJson);
  for (const o of ops) if (!handles.has(o.geom)) throw new Error('unknown handle');
  return fake('bool', kind, operandsJson);
}
export function op_cascade_read(key) {
  return JSON.stringify({ present: true, value: 0.5 });
}
export function op_log(level, message) { globalThis.__mockLogs.push(`[${level}] ${message}`); }
export function odm_web_start() { throw new Error('not in node'); }
"#;

#[test]
fn bundle_runs_in_node() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node not found; skipping bundle-in-node test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("root.js"), ROOT_JS).unwrap();
    let snapshot = odm_build::scan_project(&project).unwrap();
    let bundle = odm_export::bundle_for_tests(&snapshot).unwrap();

    let runtime = concat!(env!("CARGO_MANIFEST_DIR"), "/../odm-web/static/runtime.js");
    std::fs::write(dir.path().join("bundle.js"), bundle).unwrap();
    std::fs::copy(runtime, dir.path().join("runtime.js")).unwrap();
    std::fs::write(dir.path().join("odm_web.js"), MOCK_WASM).unwrap();
    std::fs::write(dir.path().join("driver.mjs"), DRIVER).unwrap();

    let out = Command::new("node")
        .arg(dir.path().join("driver.mjs"))
        .output()
        .expect("run node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("all checks passed"),
        "node driver failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
}

// Link the workspace stack dynamically (see odm-dylib).
use odm_dylib as _;
