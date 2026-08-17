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
- **Kernel: Manifold in the SAME wasm module — resolved by spike**
  (2026-08-17, `spikes/web-fit/`). `manifold-csg-sys` ships an
  `unstable-wasm-uu` feature building Manifold + Clipper2 for
  wasm32-unknown-unknown with plain clang + wasm-ld against wasm-cxx-shim
  — no emscripten, no second module, no glue layer. odm-kernel (whole
  Rust logic included) + odm-store + odm-ir + jsonschema link into one
  module; odm-kernel now forwards the feature (`wasm-uu`, with
  `default-features = false`). Measured: 497 KB release wasm for kernel +
  core crates; boolean perf ~1.3× native (sphere-subtract 256 segs:
  63.8 ms wasm vs 48.7 ms native, non-parallel both sides); native and
  wasm agreed on exact f64 volume bits on a sin/cos-exercising probe —
  promising for native↔web output equivalence / conformance testing
  (one sample, not a proof; no longer load-bearing since no baked data
  ships).
  Caveats of the wasm lane: upstream calls it provisional; built with
  `-fno-exceptions` (a C++ throw traps — verify odm-kernel's error paths,
  e.g. weld NotManifold, surface as status codes, not exceptions);
  `MANIFOLD_PAR=OFF` (fine for main-thread MVP); OBJ I/O off (unused).
- **Renderer: ours, via wgpu on WebGPU. No three.js.** The invariant is
  one renderer of record with one code path; a three.js rewrite means
  re-implementing depth peeling, the line-quad/AA/grid-fade path, flat
  shading, premultiplied compose — then maintaining drift forever.
  Nothing in the pass structure is WebGPU-hostile (Rgba16Float blending
  is core, WGSL has `@invariant`). WebGPU-only at first; evaluate wgpu's
  WebGL2 backend later only if reach demands it.
  Pixels won't be bit-identical to a desktop GPU — same cross-adapter
  policy as native golden PNGs (none exist for the same reason).
- **egui on web** for the UI, keeping the theme. Decided against a
  DOM/HTML chrome (2026-08-17): the viewer core
  (the `odm-viewer-core` crate) is shared with desktop, so tree,
  click-select, console pane, toggles, and future viewer features come
  from one frontend — a DOM chrome would be a second frontend needing
  permanent feature-parity work, the UI analogue of the three.js
  renderer rewrite this plan already rejects. Force integer
  `pixels_per_point` (fractional DPR blurs the bitmap fonts — same rule
  as native). `SlowIdle`/winit machinery is desktop-only; eframe's web
  backend has its own loop and our repaint-on-demand style fits it.
- **Threading: MVP is main-thread**, accepting jank during builds —
  including the initial build on page load (no baked data ships; see
  bundle format). Show build progress/loading state instead of a frozen
  page. Cancellation is a no-op on web (the main thread can't interrupt
  a build anyway; runaway build = reload) — the executor seam's cancel
  handle does nothing. Eventual: engine in a Worker, flattened scene
  posted to the main thread. Don't build the worker split into the MVP.

## Export bundle format

Static directory, no server smarts required:

- `index.html` + viewer JS glue
- ONE wasm module: egui + odm-render + odm-store + odm-build +
  odm-kernel + Manifold (kernel+core alone measured 497 KB release)
- `bundle.js` — factories for every doohickey + the framework + the
  project's API-version surfaces (the bundler must respect `//! odm <v>`
  per-file surface selection, same as snapshot install order)
- manifest: exported view (path; default = `root.js`), initial
  inputs/cascade, and export-time-extracted metas (so the web runtime
  never needs `extract_export`). MVP is exactly one view, no presets —
  presets/tabs are phase 4 (see open questions)

**No baked build output ships** (decided 2026-08-17): the client builds
everything from source on load. A pre-warmed memo snapshot was
considered and rejected — it masks a broken client build path until the
user changes a param, so the one path that matters would go unexercised
by default. Cold start makes client-side breakage/drift show
consistently, and deletes snapshot serialization, loading, and
native↔wasm hash-consistency concerns from the MVP. First paint on
heavy projects becomes a loading-progress problem, not a correctness
one. Revisit only as an explicit opt-in flag if load time proves
painful in practice.

## Template build & lookup

The bundle splits into two halves with different lifecycles:

- **Project-independent** (the *web export template*): the wasm module,
  `index.html`, viewer glue JS. Depends only on the ODM version.
- **Project-specific**: `bundle.js` and the manifest — produced by
  native code at export time; no wasm toolchain involved.

The template is built by a separate command (`xtask build-web-template`
or similar), **never** as part of a normal engine build — the wasm lane
needs the exotic clang + wasm-ld + wasm-cxx-shim toolchain and would
break every build that doesn't care about web export. Lookup at export
time: explicit `--template <dir>`/env override → `target/web-template/`
in a dev checkout → `~/.local/share/odm/web-template/<version>/` for
installed ODM. Missing template = a clear error naming the command that
builds it (or where to fetch the release artifact) — never a silent
attempt to compile wasm. Releases ship the template alongside the
binary, or embed it behind an off-by-default cargo feature CI enables.

The version stamp is a **content hash** over the template's inputs (the
wasm-side crates + framework JS), not a semver string: the template's
build/framework semantics must match the engine that authored the
project, so a mismatched template is the packaging-layer form of the
drift-between-hosts risk. Export refuses on mismatch by default
(`--force` to override); in a dev checkout the same check doubles as
staleness detection ("rebuild the template first").

Exported page behavior = one viewer tab: input panel from the
fall-through report, `t` transport when a ranged `t` falls through,
tree + click-select, orbit/pan/zoom, wireframe/x-ray toggles — all from
the shared viewer core, not web-specific code. The rebuild loop is
degenerate on a synchronous main thread: build; if inputs changed
meanwhile, build once more with the newest values. No port of the
desktop background-build machinery.

## Phases

**Phase 0 — spikes (decide feasibility, throwaway code):**

- *Manifold-to-wasm*: **DONE, green** (2026-08-17, `spikes/web-fit/` —
  kept as reference until phase 2 folds it in and deletes it). One module via clang + wasm-cxx-shim; numbers and
  caveats under "Core decisions". The spike also proved the ops seam and
  executor round trip beyond what this phase asked: the real, unmodified
  framework JS (version manifest install + determinism prelude) ran a
  factory-wrapped doohickey through `__odm.runBuild` against wasm-backed
  ops in node — CSG, volume, op_log console capture, frozen Date, IR
  JSON referencing wasm-built solids, byte-identical rebuild from a
  fresh factory invocation — and wasm→JS→wasm reentrancy
  (`scheduler_build` → imported `js_run_build` → op exports) works, so
  the native `run_build` control flow maps 1:1. Mesh data crossed as a
  `Float64Array` view of wasm memory.
  Toolchain on this machine: no root libc++/lld; extracted headers +
  wasm-ld live at `~/.local/opt/wasm-cxx/` via
  `WASM_CXX_SHIM_LIBCXX_HEADERS`/`WASM_CXX_SHIM_WASM_LD` (see the spike
  README).
- *egui+wgpu web hello*: **DONE, green** (2026-08-17,
  `spikes/web-hello/` — kept as reference until the web runtime folds it
  in). Chromium 151 / Vulkan / RADV, `Backends::BROWSER_WEBGPU` forced
  (no fallback): full pass structure validates — depth peel, Rgba16Float
  blend, line-quad wires/grid with fade, overlays, supersample 1×/2×/4× —
  no validation errors, correct layering. Bitmap fonts crisp with
  `pixels_per_point` rounded to integer. eframe web +
  `Renderer::with_device` + the viewer's OffscreenTarget structure map
  1:1. 8.0 MB wasm at `opt-level="s"` before any size pass.
  `wasm-bindgen-cli` must match the Cargo.lock pin (0.2.126).

Phase 0 is complete: both spikes green — committed to the plan.

**Phase 1 — seams in existing code** (desktop-neutral refactors, each
landable alone):

- JS-executor seam in odm-build: trait covering what `scheduler.rs`
  uses from odm-js (`run_build`, `extract_export`, cascade hashing,
  cancellation handles); native impl = today's behavior; odm-js becomes
  an optional/feature dep so odm-build compiles for wasm without V8.
  Pragma/doc parsing (`sources.rs`) moves somewhere V8-free.
- Viewer-core extraction: **DONE** (2026-08-17, the `odm-viewer-core`
  crate — see notes/architecture.md). Its `Engine` trait (set_view /
  published / store / raycast / set_selection) is exactly what the web
  host implements in phase 2.

**Phase 2 — web runtime:**

- Exporter's bundler: wrap modules, resolve the fixed import graph,
  per-version surface install, determinism prelude.
- Web ops backend implementing the `ops()` surface against the wasm
  module (wasm-bindgen exports; the spike's hand-rolled glue shows the
  shape); dep recording and invoke flow through the same odm-build code
  as native.
- Browser build loop: input-panel events → set values → rebuild
  (latest-wins), last-good scene + error/console panels like desktop.
- **Fold in and delete `spikes/web-fit/`**: it exists only as reference
  for this phase. Move what's reusable into real homes — toolchain
  setup (`WASM_CXX_SHIM_*` env vars, rootless clang/wasm-ld layout)
  into the template xtask + its docs; the ops-glue and factory-wrapping
  shape into the exporter/web runtime; the native-vs-wasm probe into a
  conformance test if kept at all — then remove the directory (and its
  mention in `notes/`).

**Phase 3 — the `export` command + site shell:** the `export` command
(engine-side or standalone — see open questions); writes the bundle
directory. Wire the viewer core to the web host;
manifest/initial-view handling; a `--serve`-less README note that any
static file server works (wasm needs correct MIME; document
`python -m http.server` caveat if any).

**Phase 4 — polish (each optional, demand-driven):** worker split,
multiple exported tabs, WebGL2 fallback, size budget pass (kernel+core
measured 497 KB; egui+wgpu will dominate — expect a few MB total),
supersample control on the page.

## Risks

1. ~~**Toolchain clash**~~ — resolved: no emscripten anywhere, one
   module (see "Core decisions"). Residual: the wasm-uu lane is
   provisional upstream, and `-fno-exceptions` means any C++ throw is a
   trap — test odm-kernel's error paths (weld failure, degenerate
   booleans) under wasm before trusting them.
2. ~~**Viewer entanglement**~~ — resolved: the read side lives in
   `odm-viewer-core` behind the `Engine` trait (2026-08-17); only the
   desktop chrome still touches `EngineState`.
3. **wgpu-on-WebGPU gaps** — believed none for our passes; spike
   verifies.
4. **Drift between hosts** — two ops backends and two executors can
   diverge. Mitigation: identical framework JS (spike-verified: ships
   byte-identical, `Deno.core.ops`-shaped glue is enough), shared
   odm-build code, and (later) running the conformance suite against
   the web runtime in CI-with-browser if that ever exists.

Executor-seam facts from the spike (sizes phase 1): odm-ir, odm-store,
odm-kernel compile for wasm32 untouched (no threads/fs/time anywhere);
jsonschema compiles for wasm too. odm-build's only wasm-hostile spots
sit exactly on the V8 side of the planned seam — `Pass::cancel`'s
isolate-terminating watchdog thread (scheduler.rs) and the
`Instant::now` BuildStats timing around `run_build`; `registry.rs`'s
Condvar compiles and its wait path is unreachable single-threaded. The
web runtime likely needs only `run_build`: meta extraction
(`extract_export`) can happen at export time, with extracted metas
shipped in the manifest.

## Open questions

- ~~Kernel module boundary~~ — resolved: one module (spike).
- Where `export` runs: engine command vs. standalone CLI mode. With no
  memo pre-warm the hot-store rationale is gone; standalone needs
  sources + framework + template only (no socket protocol addition,
  works in CI), and a broken project just shows its error in the
  exported page like it would in the viewer. Engine-side offers little
  now — lean standalone, decide in phase 3.
- Multiple tabs/views per export, and whether the manifest should carry
  presets as the page's "scenes" menu.
- Mobile/touch: egui touch support exists; orbit/pinch mapping —
  phase 4 at the earliest.
