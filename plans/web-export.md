# Web export

`odm export --web <out-dir>`: turn a project into a static site — an
*interactive* viewer (viewport, tree, input panel, `t` transport), same as
the desktop viewer's read side. Explicitly not an alternative interface: no
editing, no agent, no CLI, no chat.

## Why this is smaller than it sounds

An export is a **frozen generation**. That deletes most of the engine
before we start: no sync/watcher/hot reload, no sockets, no generation
lifecycle, no GC pressure (bounded session; at most clear-all like the
memo cache). What must run in the browser is exactly the pure core —
store + memo + build orchestration + kernel + renderer + input panel —
and that core already exists as crates with the engine glue on top.
Web export is a second, much smaller host around the same crates, not a
rewrite.

## Core decisions

- **Doohickey JS runs natively in the browser** (no V8-in-wasm). The two
  things isolates give us natively are recovered by **export-time
  bundling**, since the full file set is known when exporting:
  - *Sync module loading*: every doohickey + the framework surface is
    wrapped in a factory function (what any bundler does); "load module"
    becomes a table lookup, so the wasm build engine never needs to await
    an `import()`. This is what kills the async problem — wasm can't
    block, and browser module loading is async, but a pre-resolved bundle
    makes the whole build path synchronous like native.
  - *Fresh module state per build*: re-invoke the factory for a fresh
    module scope, mirroring the per-build disposable isolates.
  - Determinism: apply `framework/runtime/determinism.js` (frozen
    Date/Math.random) to the page realm.
  - Isolation is *weaker* than isolates (globals are shared, a hostile
    doohickey could smuggle state). Accepted and documented: an export is
    a replay of a project authored under the real engine's enforcement.
- **The ops seam is already one object.** `framework/odm/index.js`
  funnels every engine call through one `ops()` accessor reading
  `globalThis.Deno.core.ops`. The web runtime supplies a compatible ops
  object backed by wasm exports — same names, same signatures, so the
  framework JS ships identical. (Generalize the accessor so it doesn't
  literally probe `Deno`, keep the error message.)
- **Rust core compiles to wasm32-unknown-unknown**: odm-ir, odm-store,
  odm-build (pure Rust + blake3 + jsonschema). odm-js does *not* — it is
  replaced on web by the bundle runner. odm-build needs a JS-executor
  seam (see phase 1): today `scheduler.rs` calls
  `odm_js::{run_build, extract_export}` directly and Cargo-depends on
  odm-js.
- **Kernel: Manifold in wasm, probably as a second module.** Manifold is
  C++ (emscripten), egui/wgpu web is wasm-bindgen (wasm32-unknown-unknown);
  the toolchains don't link into one module happily. ManifoldCAD.org is
  the existence proof that Manifold-in-wasm works in production. Baseline
  shape: kernel wasm module (emscripten) + viewer/core wasm module
  (bindgen), JS glue between, mesh data crossing as typed arrays,
  Hash→Manifold cache living kernel-side. odm-kernel's Rust logic (weld
  diagnosis, raycast fraction conversion, clearance tiers, segment
  rules) via `wasm32-unknown-emscripten` into the kernel module, or —
  worst case — ported to the glue. **Spike first; this is risk #1.**
- **Renderer: ours, via wgpu on WebGPU. No three.js.** The invariant is
  one renderer of record with one code path; a three.js rewrite means
  re-implementing depth peeling, the line-quad/AA/grid-fade path, flat
  shading, premultiplied compose — then maintaining drift forever.
  Nothing in the pass structure is WebGPU-hostile (Rgba16Float blending
  is core, WGSL has `@invariant`). WebGPU-only at first; evaluate wgpu's
  WebGL2 backend later only if reach demands it.
  Pixels won't be bit-identical to a desktop GPU — same cross-adapter
  policy as native golden PNGs (none exist for the same reason).
- **egui on web** for the UI, keeping the theme. Force integer
  `pixels_per_point` (fractional DPR blurs the bitmap fonts — same rule
  as native). `SlowIdle`/winit machinery is desktop-only; eframe's web
  backend has its own loop and our repaint-on-demand style fits it.
- **Threading: MVP is main-thread**, accepting jank during rebuilds,
  mitigated by shipping a **pre-warmed memo cache** for the exported
  view's defaults (store is content-addressed; near-free at export
  time, instant first paint). Eventual: engine in a Worker (which is
  also a natural home for the emscripten module), flattened scene
  posted to the main thread. Don't build the worker split into the MVP.

## Export bundle format

Static directory, no server smarts required:

- `index.html` + viewer JS glue
- viewer/core wasm (egui + odm-render + odm-store + odm-build)
- kernel wasm (Manifold + odm-kernel)
- `bundle.js` — factories for every doohickey + the framework + the
  project's API-version surfaces (the bundler must respect `//! odm <v>`
  per-file surface selection, same as snapshot install order)
- store snapshot: the generation's source hashes + memo entries and
  content objects for the exported view at default inputs
- manifest: exported view (path; default = `root.js`), initial
  inputs/cascade, presets

Exported page behavior = one viewer tab: input panel from the
fall-through report, `t` transport when a ranged `t` falls through,
tree + click-select, orbit/pan/zoom, wireframe/x-ray toggles. Rebuilds
are latest-wins debounced like the desktop build loop.

## Phases

**Phase 0 — spikes (decide feasibility, throwaway code):**

- *Manifold-to-wasm*: one boolean running in a browser, called through
  odm-kernel's op surface shape. Decides: emscripten vs clang/wasi-sdk,
  one module vs two, whether odm-kernel Rust rides along via
  `wasm32-unknown-emscripten` or gets a thin glue port. Also measure
  wasm size and a ~100k-tri boolean's time vs native.
- *egui+wgpu web hello*: our theme + bitmap fonts + one mesh on WebGPU;
  check font crispness at integer scaling, and that the depth-peel
  pass structure validates on a browser device (limits: peel textures,
  Rgba16Float blend).

Both spikes green → commit to the plan; either red → rethink (three.js
fallback only becomes a question if the *renderer* spike fails, which is
the unlikely one).

**Phase 1 — seams in existing code** (desktop-neutral refactors, each
landable alone):

- JS-executor seam in odm-build: trait covering what `scheduler.rs`
  uses from odm-js (`run_build`, `extract_export`, cascade hashing,
  cancellation handles); native impl = today's behavior; odm-js becomes
  an optional/feature dep so odm-build compiles for wasm without V8.
  Pragma/doc parsing (`sources.rs`) moves somewhere V8-free.
- Viewer-core extraction from odm-engine: viewport + tree + input panel
  + transport + theme, parameterized over a small engine interface
  (submit view, read published result/report), with the desktop app as
  the first consumer. The input panel is already a pure render of
  (report, tab values), which is the right shape.
- Store snapshot serialization: write/read a generation + memo subset
  as files (also independently useful for debugging).

**Phase 2 — web runtime:**

- Exporter's bundler: wrap modules, resolve the fixed import graph,
  per-version surface install, determinism prelude.
- Web ops backend implementing the `ops()` surface against the two wasm
  modules; dep recording and invoke flow through the same odm-build
  code as native.
- Browser build loop: input-panel events → set values → rebuild
  (latest-wins), last-good scene + error/console panels like desktop.

**Phase 3 — the `export` command + site shell:** CLI command on the
running engine (it has the store hot and can pre-warm the memo cache);
writes the bundle directory. Wire the viewer core to the web host;
manifest/initial-view handling; a `--serve`-less README note that any
static file server works (wasm needs correct MIME; document
`python -m http.server` caveat if any).

**Phase 4 — polish (each optional, demand-driven):** worker split,
multiple exported tabs, WebGL2 fallback, size budget pass (expect
~5–15 MB total wasm; fine for an export, worth measuring), supersample
control on the page.

## Risks

1. **Toolchain clash** (emscripten C++ vs bindgen Rust) — the one
   genuine unknown; phase 0 exists to kill it early.
2. **Viewer entanglement** — viewer code touches `EngineState`
   directly; extraction is real work but improves desktop layering.
3. **wgpu-on-WebGPU gaps** — believed none for our passes; spike
   verifies.
4. **Drift between hosts** — two ops backends and two executors can
   diverge. Mitigation: identical framework JS, shared odm-build code,
   and (later) running the conformance suite against the web runtime in
   CI-with-browser if that ever exists.

## Open questions

- Kernel module boundary: if the spike finds Manifold builds for
  wasm32-unknown-unknown with clang (no emscripten), everything links
  into ONE module and the glue layer disappears — worth an afternoon of
  the spike before accepting two modules.
- Where `export` runs: engine command (chosen above for the hot store)
  vs. standalone CLI mode — revisit if engine-side proves awkward.
- Memo-cache snapshot size on real projects (a big animation sweep
  could bloat it; maybe cap to the default-inputs pass only).
- Multiple tabs/views per export, and whether the manifest should carry
  presets as the page's "scenes" menu.
- Mobile/touch: egui touch support exists; orbit/pinch mapping —
  phase 4 at the earliest.
