# Web export (as built, 2026-08-17)

`odm export --web <out-dir> [<project-dir>] [--view <path>] [--template
<dir>] [--force]` turns a project into a static site: the desktop viewer's
read side (viewport, tree, input panel, `t` transport, console, View menu)
around a frozen generation, built client-side. No editing, no agent, no
sockets. Verified interactive in Chromium (WebGPU/Vulkan): initial build,
transport play, input edits → rebuilds, click-select, error + weld-failure
paths all behave like the desktop.

Also in the viewer: File ▸ Export Web… (viewer/export.rs) browses for a
destination and exports the *active tab's* view — path plus set args/cascade,
which `ExportOptions.view` (a full `odm_build::View`) now carries into the
manifest; the CLI's `--view` is just `View::of(path)`. The dialog runs the
export on a background thread (meta extraction can block 10s per broken
file) and passes the process's one `JsEnv` via `export_web_with_env` —
`JsEnv::new` aborts the process if another thread is executing JS. It
refuses a destination inside a project (the site's .js files would be
scanned as doohickeys).

## Shape

Two halves with different lifecycles:

- **Project-specific** (`bundle.js` + `manifest.json`), written by
  `odm-export` at export time — native code only.
  - `bundle.js`: every framework module + every doohickey, factory-wrapped
    (`function (__req, __exp) {…}`) by the transformer in
    odm-export/src/transform.rs — a small JS lexer (comments/strings/
    templates/regex-heuristic, top-level depth tracking), NOT a parser; it
    handles the whole static import/export grammar over identifier
    bindings and errors (naming the file) on the rest (dynamic `import()`,
    `import.meta`, destructuring exports). Imports resolve at export time
    to bundle ids (doohickeys: only 'three'/'odm', per API version —
    same contract and error text as the engine loader). Also carries the
    determinism prelude as a re-runnable function and the per-version
    bare-import tables (mirror of odm-js snapshot.rs — extend both when a
    version is cut).
  - `manifest.json`: stamp, project name, initial view, and per file:
    content hash, api (or apiError), description, and the raw extracted
    `meta` export wrapped as `{"value": …}` (absent = no export;
    `metaError` = extraction failure replayed verbatim client-side).
    Metas are extracted natively at export (JsEnv), so the web runtime
    never needs `extract_export`.
- **Project-independent** (the *template*): ONE packed file,
  `web-template.bin` — `index.html`, `runtime.js`, `odm_web.js` +
  `odm_web_bg.wasm` (wasm-bindgen output of odm-web) behind a JSON header
  carrying the stamp (format: odm-export's `template` module, shared with
  xtask). Built ONLY by `cargo xtask build-web-template` into
  `target/web-template.bin` — never as part of a normal build. The name is
  static everywhere it lives, so installing replaces rather than
  accumulates; the stamp inside gates *use*, not lookup. Lookup at export:
  `--template`/`ODM_WEB_TEMPLATE` (a file) →
  `<exe>/../../web-template.bin` (dev checkout) →
  `~/.local/share/odm/web-template.bin` — which is where
  `scripts/install.sh` puts it (it runs the xtask and installs the file;
  a stamp mismatch fails the export naming the rebuild commands,
  `--force` overrides).

**Stamp**: `odm_export::TEMPLATE_STAMP`, a blake3 over framework/ + the
wasm-side crate sources + odm-export itself (the bundle format couples
exporter and template), baked by odm-export/build.rs with rerun-if-changed.
Export refuses on mismatch (`--force` overrides); in a dev checkout that
doubles as "rebuild the template first".

## The wasm host (crates/odm-web)

Workspace member whose contents are cfg'd to wasm32 — native builds see an
empty stub, so `cargo test --workspace` never needs the exotic toolchain.
Deps: odm-build with default-features=false (kernel `parallel` off),
odm-kernel with `wasm-uu`.

- **Executor seam**: `WebExecutor` implements `odm_build::Executor`.
  `run_build` pushes a session frame (thread-local stack, LIFO like
  isolates), calls `__odmWeb.runBuild(path, api, argsJson, declsJson)` in
  runtime.js, interns the returned IR JSON via `node_from_json`.
  `extract_export` answers `meta` from the manifest table. Ops
  (`op_*` wasm-bindgen exports in executor.rs) mirror odm-js ops.rs —
  keep the two in sync when adding ops; structured args cross as JSON
  strings, mesh data as typed arrays. Reentrancy (op_invoke → scheduler →
  executor → JS) works through wasm-bindgen; the invoker is taken out of
  the frame around the nested build (RefCell discipline).
- **runtime.js**: module registry (framework namespaces instantiate once
  per page — the graph is acyclic, checked implicitly by a cycle guard;
  doohickey factories re-invoked per build = fresh module scope), ops glue
  (`globalThis.__odmOps`, the generalized `ops()` accessor in
  framework/odm/index.js), and per-build realm swap: save Date/
  Math.random/console, reset Date to the page's real one, re-run the
  determinism prelude (fresh PRNG seed — isolate parity, including across
  nested builds), install the version surface, restore all three after.
  Envelope back to wasm: `{ok: ir}` or `{error: {kind, message}}`.
- **Host/app**: `WebEngine` builds a `ProjectSnapshot` from the manifest
  (code fields empty — only hashes matter for memo keys), one frozen
  generation, `BuildEngine` + the real scheduler/memo/report code.
  Degenerate build loop: `Engine::set_view` marks pending; each frame
  start runs pending synchronously (latest-wins), publishes with
  last-good root semantics + `input_report`, pins roots + gc. `app.rs` is
  the desktop chrome minus everything editable: menu bar (View only),
  tree, inputs, status band + transport (the web app keeps its own status
  band; the desktop viewer dropped its), output dock, viewport; integer
  pixels_per_point forced.

## Isolation & drift (accepted, by design)

- Weaker than isolates: page globals are shared; a hostile doohickey could
  smuggle state across builds (deep prototype mutation). An export is a
  replay of a project authored under the real engine's enforcement.
- Failure *messages* differ from native (browser JS stacks vs V8 format);
  successful builds are byte-equivalent by construction (same scheduler,
  same kernel, shared framework JS; the phase-0 spike measured bit-exact
  f64 volume agreement native↔wasm on a trig-heavy probe).
- Two ops backends (odm-js ops.rs / odm-web executor.rs) and the bundler's
  version tables (odm-export bundle.rs / odm-js snapshot.rs) can drift —
  they are deliberate mirrors; grep for the cross-references when touching
  either. The bundle-in-node test (odm-export/tests/node_bundle.rs) drives
  the REAL runtime.js + framework + a generated bundle through
  `__odmWeb.runBuild` against a mock wasm module: byte-identical rebuilds,
  fresh module state, frozen Date, cascade reads, console capture.

## Toolchain (the exotic part)

The Manifold wasm lane (`manifold-csg-sys` `unstable-wasm-uu`, forwarded
as odm-kernel `wasm-uu`) builds C++ for wasm32-unknown-unknown with plain
clang + wasm-ld against wasm-cxx-shim — no emscripten, one module. On this
machine (no root libc++/lld):

```sh
cargo xtask build-web-template   # picks up ~/.local/opt/wasm-cxx itself
```

xtask fills WASM_CXX_SHIM_LIBCXX_HEADERS / WASM_CXX_SHIM_WASM_LD in for the
wasm build from `~/.local/opt/wasm-cxx/{libcxx-headers,wasm-ld}` when they
are unset; set them by hand to point elsewhere (or `pacman -S libc++ lld`
with root and neither is needed).

`wasm-bindgen-cli` must match the Cargo.lock pin (xtask checks and says
the install command). The wasm lane is target-gated in the -sys build
script, so the feature being enabled in native feature unification is
harmless. Caveats that stand: upstream calls the lane provisional; C++ is
built `-fno-exceptions`, so an actual C++ throw traps — odm-kernel's Rust
error paths (weld NotManifold diagnosis verified in-browser) are fine, but
an unexpected Manifold-internal throw would be a wasm trap, not an error.
`MANIFOLD_PAR=OFF` (main-thread MVP).

## Testing an export

```sh
cargo xtask build-web-template
cargo run -p odm -- export --web /tmp/site examples/piston
(cd /tmp/site && python -m http.server 8742)
chromium --headless=new --no-sandbox --enable-unsafe-webgpu \
  --enable-features=Vulkan --use-angle=vulkan --window-size=1280,800 \
  --virtual-time-budget=20000 --screenshot=shot.png http://127.0.0.1:8742/
# interactive: gui-testing skill + windowed chromium (same flags) works
```

## Not built (demand-driven, was phase 4)

Worker split (builds off the main thread; the executor seam's cancel
handle is a no-op on web until then), multiple exported tabs / presets as
a scenes menu, WebGL2 fallback, size pass (wasm is ~14 MB release at
opt-level 3 before any effort; kernel+core alone measured 497 KB in the
spike — egui/wgpu dominate; try opt-level="s" + wasm-opt first),
supersample control on the page, mobile/touch mapping.
