// Web-export page glue: the module registry over bundle.js, the ops glue
// backing the framework's `ops()` seam, and the per-build entry the wasm
// executor calls. Loaded as a module after bundle.js (a classic script).
//
// Isolation model (weaker than native isolates, by design — see
// plans/web-export.md): framework modules instantiate once per page;
// part factories re-run per build for a fresh module scope; Date /
// Math.random / console are swapped in per build and restored after.

import init, * as wasm from './odm_web.js';

const B = globalThis.__odmBundle;

// ---------- module registry ----------

function instantiate(id) {
  const m = B.modules.get(id);
  if (!m) throw new Error(`bundle is missing module ${id}`);
  if (m.ns) return m.ns;
  if (m.busy) throw new Error(`module cycle at ${id}`);
  m.busy = true;
  try {
    for (const d of m.deps) instantiate(d);
    const ns = {};
    m.fac(instantiate, ns);
    m.ns = ns;
  } finally {
    m.busy = false;
  }
  return m.ns;
}

globalThis.__odmExportStar = (exp, ns) => {
  for (const k of Object.keys(ns)) {
    if (k !== 'default') exp[k] = ns[k];
  }
};

// ---------- ops glue: framework signatures → wasm exports ----------

const f64 = (a) => (a instanceof Float64Array ? a : Float64Array.from(a));
const u32 = (a) => (a instanceof Uint32Array ? a : Uint32Array.from(a));

const opsGlue = {
  op_solid_box: (size, center) => wasm.op_solid_box(f64(size), center),
  op_solid_cylinder: (h, r1, r2, seg, center) => wasm.op_solid_cylinder(h, r1, r2, seg, center),
  op_solid_sphere: (r, seg) => wasm.op_solid_sphere(r, seg),
  op_solid_extrude: (polys, h, slices, twistDeg, scaleTop) =>
    wasm.op_solid_extrude(JSON.stringify(polys), h, slices, twistDeg, f64(scaleTop)),
  op_solid_revolve: (polys, seg, deg) => wasm.op_solid_revolve(JSON.stringify(polys), seg, deg),
  op_solid_sweep: (polys, frames) =>
    wasm.op_solid_sweep(JSON.stringify(polys), JSON.stringify(frames)),
  op_solid_from_mesh: (positions, indices) => wasm.op_solid_from_mesh(f64(positions), u32(indices)),
  op_boolean: (kind, operands) => wasm.op_boolean(kind, JSON.stringify(operands)),
  op_hull: (operands) => wasm.op_hull(JSON.stringify(operands)),
  op_transform_bake: (geom, matrix) => wasm.op_transform_bake(geom, f64(matrix)),
  op_volume: (geom) => wasm.op_volume(geom),
  op_area: (geom) => wasm.op_area(geom),
  op_bounds: (geom) => JSON.parse(wasm.op_bounds(geom)),
  op_raycast: (geom, origin, dir, maxDist) =>
    JSON.parse(wasm.op_raycast(geom, f64(origin), f64(dir), maxDist)),
  op_clearance: (a, b) => JSON.parse(wasm.op_clearance(a, b)),
  op_cascade_read: (key) => JSON.parse(wasm.op_cascade_read(key)),
  op_invoke: (path, args, cascade) =>
    wasm.op_invoke(path, JSON.stringify(args ?? null), JSON.stringify(cascade ?? null)),
  op_log: (level, message) => wasm.op_log(level, message),
};

// ---------- per-build entry (called by the wasm executor) ----------

// Captured before any build touches the realm; restored after each one so
// the page (egui glue, error reporting) never sees the frozen builds' view.
const REAL = { Date: globalThis.Date, random: Math.random, console: globalThis.console };
const apiStack = [];

function installSurface(api) {
  const v = globalThis.__odmVersions?.[api];
  if (!v) throw new Error(`bundle has no API version ${api}`);
  v.install(globalThis);
}

function formatError(e) {
  if (e instanceof Error) {
    // The V8/SpiderMonkey stack already carries frame lines; keep the first
    // few, they name part code by bundle position.
    const stack = typeof e.stack === 'string' ? e.stack.split('\n').slice(0, 9).join('\n') : '';
    return stack.includes(e.message) ? stack : `${e.message}\n${stack}`;
  }
  return String(e);
}

globalThis.__odmWeb = {
  runBuild(path, api, argsJson, declsJson) {
    const saved = { Date: globalThis.Date, random: Math.random, console: globalThis.console };
    apiStack.push(api);
    try {
      // Fresh determinism per build (isolate parity): derive the frozen
      // Date from the real one (no subclass chains) and reseed the PRNG.
      globalThis.Date = REAL.Date;
      B.prelude();
      installSurface(api);
      const d = B.parts.get(path);
      if (!d) {
        // The bundler could not transform it (bad import, unsupported
        // syntax): its builds fail with that error, like the engine's loader.
        const broken = B.broken.get(path);
        if (broken) return JSON.stringify({ error: { kind: 'js', message: broken } });
        return JSON.stringify({
          error: { kind: 'internal', message: `bundle has no part at ${path}` },
        });
      }
      const ns = {};
      // Fresh module scope, like a fresh isolate — and with the ops window
      // closed as in one, even when this build nests inside another.
      globalThis.__odm.moduleScope(() => d.fac(instantiate, ns));
      const ir = globalThis.__odm.runBuild(ns, JSON.parse(argsJson), JSON.parse(declsJson));
      return JSON.stringify({ ok: ir });
    } catch (e) {
      return JSON.stringify({ error: { kind: 'js', message: formatError(e) } });
    } finally {
      apiStack.pop();
      globalThis.Date = saved.Date;
      Math.random = saved.random;
      globalThis.console = saved.console;
      const parent = apiStack[apiStack.length - 1];
      if (parent) installSurface(parent);
    }
  },
};

// ---------- boot ----------

function fatal(message) {
  const el = document.getElementById('odm-status');
  if (el) el.textContent = message;
  REAL.console.error(message);
}

async function boot() {
  // wgpu picks WebGPU when navigator.gpu exists, WebGL2 otherwise (a scratch
  // canvas probes it — the real canvas must stay context-free for wgpu).
  if (!navigator.gpu && !document.createElement('canvas').getContext('webgl2')) {
    fatal('This page needs WebGPU or WebGL2, and this browser offers neither.');
    return;
  }
  try {
    const manifest = await (await fetch('./manifest.json')).text();
    await init();
    globalThis.__odmOps = opsGlue; // the framework's ops() seam
    // Instantiate every bundled version manifest: registers __odmVersions.
    for (const v of Object.values(B.versions)) instantiate(v.manifest);
    document.getElementById('odm-status')?.remove();
    await wasm.odm_web_start(manifest, 'odm-canvas');
  } catch (e) {
    fatal(`ODM web viewer failed to start: ${e instanceof Error ? e.message : e}`);
    throw e;
  }
}

// In a browser this file IS the page entry. The node-driven bundle test
// (odm-export tests) imports it against a mock ./odm_web.js instead and
// drives __odmWeb.runBuild directly.
if (typeof document !== 'undefined') {
  boot();
} else {
  globalThis.__odmOps = opsGlue;
  for (const v of Object.values(B.versions)) instantiate(v.manifest);
}
