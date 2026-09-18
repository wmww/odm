# Web export (as built, 2026-08-17)

`odm export --web <out-dir> [<project-dir>] [--view <path>] [--template
<dir>] [--force]` turns a project into a static site: the desktop viewer's
read side (viewport, tree, input panel with the `t` play button, console,
View menu)
around a frozen generation, built client-side. No editing, no agent, no
sockets. Verified interactive in Chromium (WebGPU/Vulkan): initial build,
transport play, input edits → rebuilds, click-select, error + weld-failure
paths all behave like the desktop. The WebGL2 lane (below) verified
headless: boot, build, full render matching the WebGPU lane.

## Rendering backends: WebGPU preferred, WebGL2 fallback — POLICY

The page requests `BROWSER_WEBGPU | GL`; wgpu picks WebGPU when
`navigator.gpu` exists and WebGL2 otherwise (browsers only expose
`navigator.gpu` in secure contexts — https or localhost — and Linux
browsers still often ship it off, so the GL lane is what most shared links
hit today). The app logs `ODM viewer: WebGPU/WebGL2 (<adapter>)` to the
browser console at startup.

Standing constraints this creates (also documented at the top of
odm-render/src/lib.rs — keep both in sync):

- **odm-render must stay inside wgpu's downlevel_webgl2 envelope.** No
  compute/storage buffers, ≤4 bind groups (we use exactly 4), fill-only
  polygon mode, no `Features::` requests, no texture view reinterpretation
  (VIEW_FORMATS), depth textures readable ONLY via comparison samplers
  (`textureSampleCompareLevel`; `textureLoad` on depth is undefined on GL
  and naga rejects it — this is why peeled_or_hidden uses LEQUAL/GEQUAL
  samplers), float targets only where EXT_color_buffer_float reaches
  (Rgba16Float accum is fine). `textureSampleCompare` (implicit-LOD) also
  trips WGSL uniformity analysis under `||` — use the Level form.
- **The final target is plain Rgba8Unorm** with sRGB encoded explicitly in
  fs_compose/fs_downsample (no sRGB view reinterpretation anywhere); egui
  samples those gamma bytes directly, PNG readback gets them unchanged.
- **Ship both backends.** Size is a non-argument: egui-wgpu's default
  features compile both lanes anyway. Measured (bindgen'd wasm, release):
  both 14.13 MB (4.84 MB gz); webgpu-only 10.72 MB (3.73 MB gz) — the GL
  lane (wgpu-core/hal + naga's GLSL writer) is the ~3.4 MB; webgl-only
  saves only ~0.2 MB over both. Dropping the fallback is the only real
  size lever, and we're not taking it.
- Renderer changes must be tested on BOTH lanes (recipe below); GL
  failures are often silent black, not validation errors.

Also in the viewer: File ▸ Export Web… (viewer/export.rs) browses for a
destination (starting in the project; default folder `web-export`) and
exports the *active tab's* view — path plus set args/cascade, which
`ExportOptions.view` (a full `odm_build::View`) now carries into the
manifest; the CLI's `--view` is just `View::of(path)`. The dialog runs the
export on a background thread (meta extraction can block 10s per broken
file) and passes the process's one `JsEnv` via `export_web_with_env` —
`JsEnv::new` aborts the process if another thread is executing JS.

Sites may live inside the project: every export writes
`odm_build::EXPORT_MARKER` (`.odm-export`, written first) into the out dir,
and the project scanner skips marked dirs. `check_destination` (odm-export
lib.rs, shared by CLI and dialog) still refuses a project *root* and an
unmarked dir inside a project that already holds `.js` files (a stale
pre-marker export must be deleted once; the error says so). A doohickey the
bundler cannot transform (bad import etc.) no longer fails the export: it
ships in `B.broken` (path → bare message) instead of a factory, export
warns, and the page fails that doohickey's builds with the message —
engine-like per-file failure.

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
  `target/web-template.bin` — never as part of a normal build — and
  **embedded** into odm by odm-export's build.rs (`include_bytes!` via
  OUT_DIR, rerun-if-changed on that file; a checkout with no template gets
  an empty placeholder there, because cargo treats a *missing*
  rerun-if-changed path as always-dirty and would rebuild the world each
  time). So: xtask, then rebuild odm, and the installed binary is
  self-contained (`scripts/install.sh` does both in that order). An empty
  embed fails the export with "built without the web export template".
  `--template`/`ODM_WEB_TEMPLATE` (a file) override the embedded copy — for
  iterating on the wasm without relinking odm, and what the web lane test
  uses. A stamp mismatch fails the export naming the rebuild commands,
  `--force` overrides.

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
  the desktop chrome minus everything editable: menu bar (View only), the
  same right-hand side bar (inputs above tree, draggable split), the `t`
  transport in its own bottom panel when the view has one, output dock,
  viewport; integer pixels_per_point forced. No status band — it went the
  way the desktop viewer's did (2026-08-17); the console says what the
  build had to say.

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

## Testing an export (BOTH lanes)

**Automated: `cargo xtask test-web`** — builds the template, then runs the
`#[ignore]`d `crates/odm-export/tests/web_lane.rs` against it. It exports
the piston example, serves it on a free port, and loads it in headless
chromium on both lanes (GL by injecting the `navigator.gpu` override into
a copied `index.html`), asserting: the `ODM viewer: <lane>` line names the
lane expected, no console errors, the screenshot is not one flat colour
(a GL failure is silently black), and — the one property that is really
the web lane's own — that the `ODM root: <hex>` line the host publishes
equals the root hash `odm_build` produces natively for the same view.
Never in `cargo test --workspace`: the wasm build alone dwarfs the suite.

The manual recipe, for looking at it:

```sh
cargo xtask build-web-template
cargo run -p odm -- export --web /tmp/site examples/piston
(cd /tmp/site && python -m http.server 8742)
# WebGPU lane:
chromium --headless=new --no-sandbox --enable-unsafe-webgpu \
  --enable-features=Vulkan --use-angle=vulkan --window-size=1280,800 \
  --virtual-time-budget=20000 --screenshot=shot.png http://127.0.0.1:8742/
# WebGL2 lane: copy the site and hide navigator.gpu before the scripts —
#   <script>Object.defineProperty(Navigator.prototype,"gpu",{value:undefined});</script>
# in index.html above the bundle.js tag, then (SwiftShader is fine):
chromium --headless=new --no-sandbox --enable-unsafe-swiftshader \
  --window-size=1280,800 --virtual-time-budget=20000 \
  --screenshot=shot-gl.png http://127.0.0.1:8743/
# --enable-logging=stderr shows the "ODM viewer: <lane>" console line.
# interactive: gui-testing skill + windowed chromium (same flags) works
```

Last cross-check (piston, 2026-08-17): 154 of 1,024,000 pixels differed
>2/255 between lanes (edge AA between different rasterizers) — treat a
bigger divergence as a bug.

## Not built (demand-driven, was phase 4)

Worker split (builds off the main thread; the executor seam's cancel
handle is a no-op on web until then), multiple exported tabs / presets as
a scenes menu, size pass (wasm is ~14 MB release at opt-level 3 before
any effort; kernel+core alone measured 497 KB in the spike — egui/wgpu
dominate, see the backend-size table above; try opt-level="s" +
wasm-opt first), supersample control on the page, mobile/touch mapping.
