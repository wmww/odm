// Web-export fit spike runner. Same WebAssembly API in node and browsers.
//
// Usage:
//   cargo build --target wasm32-unknown-unknown   (see ../../plans/web-export.md; needs
//     WASM_CXX_SHIM_LIBCXX_HEADERS / WASM_CXX_SHIM_WASM_LD if libc++/lld aren't installed)
//   node run.mjs
import fs from 'node:fs';

const bytes = fs.readFileSync(
  new URL('./target/wasm32-unknown-unknown/debug/web_fit_spike.wasm', import.meta.url));

let exp; // set after instantiation; js_run_build re-enters through it

// The "JS executor": stands in for the bundle runner executing a doohickey's
// build(). Called FROM wasm (scheduler_build), calls ops back INTO wasm.
function js_run_build(buildId) {
  console.log(`  js_run_build(${buildId}): running doohickey JS, calling ops back into wasm`);
  const box = exp.op_solid_box(2, 2, 2);
  const ball = exp.op_solid_sphere(1.25, 32);
  // subtract the sphere, pushed up so it bites a corner
  return exp.op_boolean(1, box, ball, 1, 1, 1);
}

const { instance } = await WebAssembly.instantiate(bytes, { env: { js_run_build } });
exp = instance.exports;

exp.odm_init();

// 1) Direct ops: the seam JS-side framework code would call.
const a = exp.op_solid_box(1, 1, 1);
const b = exp.op_solid_box(1, 1, 1);
const u = exp.op_boolean(0, a, b, 0.5, 0, 0); // union, b shifted +x
const vol = exp.op_volume(u);
console.log(`union volume = ${vol} (expect 1.5)`);
if (Math.abs(vol - 1.5) > 1e-9) throw new Error('bad union volume');

// 2) Reentrant round trip: JS -> wasm scheduler -> JS executor -> wasm ops.
const root = exp.scheduler_build(7);
const tris = exp.op_tri_count(root);
const rvol = exp.op_volume(root);
console.log(`scheduler_build -> handle ${root}: ${tris} tris, volume ${rvol.toFixed(4)}`);
if (!(tris > 0) || !(rvol > 0 && rvol < 8)) throw new Error('bad reentrant build result');

// 3) Mesh data across the boundary as a typed-array view (f64, no copy JS-side).
const n = exp.op_mesh_load(root);
const pos = new Float64Array(exp.memory.buffer, exp.op_mesh_ptr(), n);
let maxAbs = 0;
for (const v of pos) maxAbs = Math.max(maxAbs, Math.abs(v));
console.log(`mesh positions: ${n / 3} vertices, max |coord| = ${maxAbs} (expect 1, f64-exact)`);
if (maxAbs !== 1) throw new Error('bad mesh data');

console.log('web-fit spike: all checks passed ✓');
