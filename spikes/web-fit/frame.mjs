// Web-export fit spike, part 2: the REAL framework JS (unmodified, straight
// from framework/) running outside V8 against wasm-backed ops.
//
// Mirrors what the export bundler + web runtime would do:
//   1. import the version manifest (registers globalThis.__odmVersions)
//   2. apply the determinism prelude to the realm
//   3. supply a Deno.core.ops-shaped object backed by the wasm module
//   4. install the surface, run a factory-wrapped doohickey via __odm.runBuild
//
// The ops glue here adapts to the spike's simplified exports (u32 handles,
// translation-only operand transforms); the real web backend implements the
// full op signatures via wasm-bindgen instead. Run: node frame.mjs
import fs from 'node:fs';

const bytes = fs.readFileSync(
  new URL('./target/wasm32-unknown-unknown/release/web_fit_spike.wasm', import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes, {
  env: { js_run_build: () => 0 }, // unused in this part
});
const exp = instance.exports;
exp.odm_init();

// --- ops glue: framework signatures -> spike exports ------------------
// Hashes are opaque strings to the framework; the glue keeps the mapping.
const handles = new Map();
const fake = (h) => {
  const s = `wasm:${h}`;
  handles.set(s, h);
  return s;
};
const logs = [];
const kinds = { union: 0, difference: 1, intersection: 2 };
const opsGlue = {
  op_solid_box: (size, _center) => fake(exp.op_solid_box(size[0], size[1], size[2])),
  op_solid_sphere: (r, segments) => fake(exp.op_solid_sphere(r, segments)),
  op_boolean: (kind, operands) => {
    if (operands.length !== 2) throw new Error('spike glue: 2 operands only');
    const [a, b] = operands;
    return fake(exp.op_boolean(
      kinds[kind], handles.get(a.geom), handles.get(b.geom),
      b.matrix[12], b.matrix[13], b.matrix[14]));
  },
  op_volume: (geom) => exp.op_volume(handles.get(geom)),
  op_log: (level, msg) => logs.push(`[${level}] ${msg}`),
};

globalThis.Deno = { core: { ops: opsGlue } };

// --- what the exported page does at startup ---------------------------
await import('../../framework/runtime/determinism.js');
await import('../../framework/versions/unstable.js');
globalThis.__odmVersions.unstable.install(globalThis);

// --- a bundled doohickey: factory per module, re-invoked per build ----
const factory = () => {
  // module scope — fresh on each invocation, like a fresh isolate
  const bite = odm.sphere(1.25, { segments: 32 }).translate(1, 1, 1);
  return {
    default: function build(_ctx) {
      const part = odm.box(2).subtract(bite);
      console.log('volume', part.volume().toFixed(4), 'at', Date.now());
      return part.color('#8888ff');
    },
  };
};

// installGlobals replaced `console` with the op_log capture, so status
// output goes straight to stdout.
const print = (...a) => process.stdout.write(a.join(' ') + '\n');
const ir = __odm.runBuild(factory(), {}, {});
print('IR out:', JSON.stringify(ir));
print('console captured:', JSON.stringify(logs));

const geomHash = ir.mesh ?? ir.geom;
if (!handles.has(geomHash)) throw new Error('IR does not reference a wasm-built solid');
if (!logs[0]?.includes('7.000') || !logs[0]?.includes('1700000000000')) {
  throw new Error('volume/determinism check failed');
}
// second build: fresh module scope, memoizable identical result
const ir2 = __odm.runBuild(factory(), {}, {});
if (JSON.stringify(ir2) !== JSON.stringify(ir)) throw new Error('rebuild not identical');
print('framework-on-wasm spike: all checks passed ✓');
