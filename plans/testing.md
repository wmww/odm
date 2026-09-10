# Testing plan

From the 2026-09-09 survey. State then: 322 tests, 0 failures, ~2.5s after
compile, no CI (deliberate: `cargo test --workspace` before every commit is
the gate). This plan removes what is vacuous or negative-value, fills the
gaps that matter, and says what we deliberately do not test.

Execute phases in order, one commit per phase (Phase 3 in several commits).
Run `cargo test --workspace` before each commit. Read `notes/architecture.md`
"Testing" and `tests/conformance/README.md` before starting.

## What makes a test worth having here

A test earns its place only if all three hold:

1. **A plausible change breaks it by accident.** Not "someone deleted the
   feature", but a refactor, a dependency bump, an unrelated fix.
2. **The breakage is a real bug a user or agent would hit.** If the honest
   response to a failure is "update the expected value", the test is a
   tax. Tests that pin where a UI element sits, the exact wording of a
   message we expect to keep polishing, or a hash of current output are
   in this class: delete, don't write.
3. **It is cheap.** The gate stays under ~10s total. Anything needing a
   browser, a wasm toolchain or a compositor is opt-in.

**The JS API is the exception to "don't pin specifics".** Once a version is
stamped, every observable behavior (including some bugs) must stay
byte-for-byte compatible for existing projects. So conformance tests pin
*exact* semantics: argument shapes, defaults, error conditions, which
operand's color wins. Values must still be *derivable* (analytic or exact
CSG arithmetic), never copied from engine output.

**Integration over unit** where the cost is similar: a test that drives the
`odm` binary or the JS API tests what people observe; a unit test of a
helper tests that the code does what it does.

**A test that cannot run must fail**, naming the escape hatch, never
silently pass.

## Phase 0: remove negative-value tests

- **Delete `example_scene_hashes_are_stable`**
  (`crates/odm-build/tests/suite/examples.rs`). It pins five pasted IR
  hashes and its own failure message says "regenerate from the printed
  values", so no failure ever leads to a fix. Cross-run determinism is
  already covered by `all_examples_build_and_are_deterministic` and
  `consistency_incremental_equals_scratch`; the examples' real properties
  are covered by `bracket_has_holes`, `assembly_shares_wheel_geometry`,
  `piston_animates_and_memoizes_static_parts` etc., which stay.
- Skim the remaining suites for the same smell (a literal that came from
  running the code, not from reasoning). The survey found no others; note
  anything found in the commit message rather than fixing it ad hoc.

## Phase 1: no vacuous passes

Today 12 render tests return early when there is no GPU adapter:
`renderer_or_skip()` in `crates/odm-render/tests/render.rs` (10 call
sites) and two inline `let Ok(mut renderer) = Renderer::new() else {
return }` in `crates/odm-render/tests/wire.rs`. On a machine without
Vulkan they all go green while testing nothing.

1. In `crates/odm-render/src/gpu.rs`, record the adapter info at
   construction (`adapter.get_info()`: name, backend, device type) and
   expose `Renderer::adapter_description(&self) -> String`.
2. Add `crates/odm-render/tests/common/mod.rs` with one helper:

   ```rust
   /// The test renderer. Panics without a GPU adapter unless
   /// ODM_TEST_NO_GPU=1 (then returns None so the caller skips).
   pub fn renderer() -> Option<Renderer>
   ```
   Panic message: `no GPU adapter: <error>. Set ODM_TEST_NO_GPU=1 to skip
   the render tests on a machine that genuinely has none.` On success,
   `eprintln!` the adapter description once per process (`OnceLock`), so
   a silent fallback to a software rasterizer is visible in `--nocapture`
   output.
3. Replace all 12 sites with `let Some(mut renderer) = common::renderer()
   else { return };`.
4. Same treatment for node in `crates/odm-export/tests/node_bundle.rs`:
   panic naming `ODM_TEST_NO_NODE=1` unless that is set. node is a real
   dependency of the web-export lane, not an optional extra.
5. Note both variables in `notes/architecture.md` "Testing".

## Phase 2: end-to-end through the `odm` binary

Nothing spawns the binary today. Untested as a result: arg dispatch in
`crates/odm/src/main.rs`, engine startup and the socket lifecycle, the
inotify watcher (`crates/odm-engine/src/watcher.rs`, 0 tests), and the
CLI to engine round trip as shipped. Measured cost (debug build): socket
up 86ms after `odm run --headless`, `status` 8ms, `render` 61ms,
post-edit `inspect` 19ms. A file with ~10 engine spawns lands under 3s.

### Setup

- New `crates/odm/tests/e2e.rs`. The `odm` package is a bin-only crate in
  `default-members`, so this test binary links nothing heavy and the
  binary path is `env!("CARGO_BIN_EXE_odm")`. (The `[[bin]] test = false`
  flag disables unit tests in the bin, not integration tests.) Add
  `tempfile`, `serde_json` and `png` as dev-dependencies of `odm`.
- **Temp projects under `/tmp`, not the scratchpad**: the socket is
  `<project>/.odm/engine.sock` and Unix socket paths are limited to ~104
  bytes. Use `tempfile::Builder::new().prefix("odm-e2e-").tempdir_in("/tmp")`.
- A project is a dir with `odm.toml` (copy `examples/hello-bracket/odm.toml`
  for the format) and doohickey files. Author tiny projects inline, e.g.
  `root.js` = `//! odm unstable\nexport default () => odm.box(10);`, so
  expected volumes are exact. Copy `examples/assembly` when parts are
  needed.
- Helper `struct Engine { child: Child, project: TempDir }` with
  `Engine::start(project) -> Engine` (spawn `odm run <dir> --headless`
  with stdout/stderr captured, then poll `UnixStream::connect(sock)` every
  10ms for up to 10s, panicking with the captured stderr on timeout) and
  `Drop` that kills and waits. The headless engine has **no signal
  handler**, so `kill()` is SIGKILL and leaves the socket file behind: that
  is fine and is itself tested below.
- Helper `fn odm(project, args) -> (exit_code, stdout_json_or_text, stderr)`
  running the binary with `--project <dir>` (or cwd set to the project).
- One engine per test (tests run in parallel threads; each has its own
  temp dir). Do not share engines across tests.

### Cases (assert structure and codes, never prose)

1. **Startup**: socket appears; `status` exits 0 and its `views` list's
   first entry reports `build: "ok"` (field names: `docs/cli.md`
   "status").
2. **Hot reload via sync-on-query** (the headline invariant): with
   `root.js` = `odm.box(10)`, `inspect '{"fields":["name","volume"]}'`
   reports volume 1000. Overwrite the file with `odm.box(20)` and, with no
   sleep and no explicit sync, the very next `inspect` reports 8000.
   Then the same for a **part**: copy `examples/assembly`, record the
   root volume, change a dimension in one `parts/*.js`, next query
   differs. Then a **rename**: rename that part file; the next query
   exits nonzero and its error names the old path (the sync must notice
   deletions, not just edits); rename back, next query is clean.
3. **The watcher, without any query** (the only place it is exercised):
   start `odm poll --follow` as a child with piped stdout and a reader
   thread collecting lines. Overwrite `root.js` with a syntax error and
   touch nothing else. Within 2s a JSON line arrives whose `builds` has an
   entry with `build: "error"`. Heal the file; within 2s a line with
   `build: "ok"` arrives. (`--follow` never syncs, so these lines can
   only come from watcher → rebuild → events.) Kill the follower at the
   end.
4. **Failure and recovery**: with a `build()` that throws
   `new Error("boom")`, `inspect` exits nonzero and the JSON carries the
   message `boom`; `status` shows that view as `build: "error"`. Because
   `meta` still evaluated, the error response carries `inputs` (see
   `docs/cli.md` "Errors"): declare one input and assert it is listed.
   Heal → clean.
5. **Render**: `render '{"out": "<tmp>/a.png", "width": 64, "height": 64}'`
   exits 0; the file decodes as a 64×64 PNG; it is not a single uniform
   color; a second identical request produces byte-identical bytes. Skip
   this one case (only) under `ODM_TEST_NO_GPU=1`, with an `eprintln!`.
6. **Chat**: `say hello` exits 0. `poll --timeout 0.2` exits 0 promptly
   with `ok: true`, `messages: []` and a `builds` array. (User messages
   enter only through the viewer, so the queue/ack semantics are covered
   by `server.rs`/`state.rs` unit tests over a real socket; this case just
   proves the commands reach a real engine.)
7. **Errors an agent will hit**, each with no engine or a second one:
   - no engine: exit code 2, stderr contains `odm run` (the fix command);
   - second `odm run --headless` on the same project: exits nonzero
     quickly, stderr contains `already running`, and the first engine
     still answers `status`;
   - unknown command, and a malformed JSON argument: nonzero exit,
     non-empty stderr or an `ok: false` JSON body (read
     `odm_cli::run` for which).
8. **Stale socket**: SIGKILL an engine (drop the guard), keep the temp
   dir, start a new engine on the same project: it comes up (`bind()`
   probes the stale socket and removes it). This is the crashed-engine
   restart every user eventually needs.
9. **`odm docs` smoke** (no engine): for every `docs/api/*.md` except
   README, every top-level `docs/*.md` except README, and `prompt`,
   `odm docs <stem>` exits 0 with non-empty stdout. `odm docs` bare lists
   them all. `odm docs search box` exits 0 and prints at least one
   section. This is the agent's documentation surface and has zero tests.

Wording is guarded elsewhere (`cli_reference_is_current` diffs
`docs/cli.md` against the parser). Do not assert message text beyond the
substrings above.

## Phase 3: the JS API (main body of work)

The API is the product surface and must be the best-tested thing here.
Two guards exist: the conformance suite (`tests/conformance/unstable/`,
15 projects, runner `crates/odm-engine/src/conformance.rs`, ~10ms per
project) and the docs doctests (~30 `js` blocks,
`crates/odm-build/tests/suite/doctests.rs`). There is room to grow the
suite several times over before speed matters.

### 3a. Runner additions (do first, small)

Read `run_check` before changing it; keep the aggregate-all-failures
behavior. Add three check kinds and document them in
`tests/conformance/README.md`:

- `{ node: 'name', color: '#rrggbb' | [r,g,b,a] | null, opacity: x | null,
  volume: [v, eps], bounds: {...} }`: address one node by name via
  `scene::locate` and assert its *authored* attributes and subtree
  measurements (`Inspector` already computes these for `odm inspect`).
- `{ flat: [[r,g,b,a], ...] }`: the multiset of *effective* per-instance
  colors after inheritance and opacity, from `odm_render::flatten_scene`,
  compared order-insensitively (sort, round to 1e-6). This is what the
  renderer will draw, so it pins inheritance end to end. If flattened
  instances do not carry enough to compare, compare colors only; that is
  sufficient.
- `{ meshes: n }`: the number of distinct mesh hashes reachable from the
  scene root. Pins "shared geometry is interned once".

### 3b. New and thickened conformance files

One file per area. Every pinned number must come with the arithmetic in a
comment. Use the `mode` enum pattern from `strict-errors.js` to pack many
error cases into one file. Where the docs are silent on a behavior,
**decide, document it in `docs/api/`, then pin it**; where the current
behavior is clearly a bug, fix it first (that is the point). Use
`ctx.input` modes rather than one file per case.

| file | pins |
|---|---|
| `hull.js` (new) | hull of two unit cubes at x∈[0,1] and x∈[10,11] is the box [0,11]×[0,1]×[0,1] → volume 11 exactly; hull of one operand equals it (volume unchanged); pending transforms participate (`a.hull(a.translate(0,0,4))` with `a`=box(2) → 2×2×6=24); the hull of eight small cubes at a big cube's corners is that cube; a non-Solid operand errors. Which operand's color survives (csg.md says first for CSG; decide for hull, document, pin with `node`). |
| `names.js` (new) | `.name()` reaches raycast `name` and `node` lookup; `name(123)` stores `'123'`; a second `name` replaces; a name survives later transforms; whether CSG results keep an operand's name (read `Solid._bool`, decide, document). Do **not** pin the known gap that a name on an `Instance` wrapper does not reach raycast (see the comment in `invoke/root.js`); file an issue for it instead. |
| `cylinder.js` (new) | `segments: 4` gives an exact square prism: volume 2r²h, tight eps; `segments: 6` → (3√3/2)r²h; cone via `r2`: πh(r²+r·r2+r2²)/3 with the inscribed band; `center: false` → z∈[0,h] exact bounds; `segments: 2` errors. |
| `sphere.js` (new) | default-segments volume band; volume is monotone in `segments` (8 < 16 < 48, all below 4/3πr³); bounds within r on every axis and exactly ±r on z (poles are vertices); negative or NaN radius errors. |
| `revolve.js` (thicken) | `angle: π` is half the full-turn volume to a loose band; `curveSegments` with a `THREE.Shape` arc profile; a profile point with x<0 errors (message per errors.md). |
| `extrude.js` (thicken) | `twist` preserves the slice cross-section (volume within 1% of untwisted); `slices` does not change an untwisted prism; a `THREE.Shape` circle profile with `curveSegments: n` has the *exact* inscribed-polygon volume (n/2)·r²·sin(2π/n)·h if `extractPoints(n)` yields n points per full arc (verify in `toPolygons`; otherwise use a band); a Shape with a hole subtracts. |
| `transforms-compose.js` (new) | `a.translate(5,0,0).subtract(b)` vs `a.subtract(b.translate(-5,0,0))` differ as predicted; `rotateZ(90°, {about: p})` equals `translate(-p).rotateZ(90°).translate(p)` (same bounds and raycast); `applyMatrix4(M)` equals the chained calls that build M; rotate∘scale and scale∘rotate differ (bounds); `scale(-1,1,1)` (a mirror) still has volume 1000 and its raycast normals point outward. If the mirror comes out inside-out, that is a real bug: fix before pinning. |
| `scale-queries.js` (new) | non-uniform scale through every query: `box(10).scale(2,1,1)` raycast from +x hits at 10; `bounds()` is [-10,10]×[-5,5]²; JS `area()` bakes the transform so it is 2·(200+200+100)=1000 (unlike the runner's `area` check, which ignores instance transforms); `clearance` between two scaled solids is exact. |
| `nesting.js` (new) | groups in groups accumulate transforms (a translated group holding a translated child: world position is the sum); arrays and nested arrays flatten; `null`/`undefined` children are dropped; `odm.group()` with nothing builds with volume 0; a `subtract` that empties a solid builds, volume 0, JS `bounds()` returns `null`. |
| `instances/` (new dir) | one Solid passed to two invokes and an Instance placed twice → `meshes: 1`; instance of an instance (root → a.js → b.js) composes transforms; a color on the Instance wrapper reaches uncolored descendants (`flat`); a cascade value set on the outer invoke reaches the inner file. |
| `colors-inherit.js` (new) | group color → uncolored child inherits, colored child overrides; opacity multiplies down the tree (`group.opacity(0.5)` over a child `.opacity(0.5)` → 0.25 effective); `.opacity(0.5).opacity(0.5)` is 0.25; color alpha × ancestor opacity; CSG keeps the first operand's color; all via `flat` plus `node` for the authored values. |
| `degenerate.js` (new) | `box(0)` and `box([1,2])`: decide (error vs empty), document, pin; `raycast` miss is `null`; `maxDist` shorter than the hit is a miss; `deg(180)` is exactly `Math.PI`; `clearance` of touching solids (already in clearance.js, leave there). |
| `three-math.js` (new) | `Vector3` accepted wherever `[x,y,z]` is (`rotate` axis, `raycast` origin/dir, `about`); `raycast` returns `{distance, point: Vector3, normal: Vector3, ...}`; `bounds()` is a `Box3` (use `getSize`); `applyMatrix4(new THREE.Matrix4().makeRotationZ(r))` equals `rotateZ(r)`; a `Quaternion`/`Euler` rotation through `Matrix4` equals `rotate(axis, r)`; `THREE.MathUtils.degToRad(90) === odm.deg(90)`. Assert equalities JS-side (throw on mismatch, like clearance.js) and expose one summary geometry to the runner. |
| `three-generators.js` (new) | `fromThreeGeometry` of Box, Cylinder, Sphere, Torus (2π²Rr² band), Lathe, Extrude generators builds; `BoxGeometry(1,2,3)` keeps three's Y-up axes (the 2 is on y: no silent axis swap, per three.md); `ShapeGeometry` errors with `open surface`. |
| `strict-errors.js` (extend) | modes for: opacity out of 0..1; unknown `extrude`/`revolve`/`sphere` option; `box` bad size shape; `cylinder` segments below 3; `ctx.invoke` of a missing path (message lists available files); `ctx.input` of an undeclared name (message lists declared names). Note: calling `odm.*` at module top level makes the module fail to evaluate, so `checks` are unreadable; pin that one as a ` ```js error="…"` doctest in `errors.md`, not here. |

Leave `errors.js`, `colors-number.js` and the other small existing files
as they are; consolidation is not worth the churn.

Memoization as seen from JS (`invoke` × cascade × `t`) is **not**
observable from a conformance check (logs are replayed on memo hits by
design) and is already pinned by `cascade_only_invalidates_readers` and
`piston_animates_and_memoizes_static_parts`. Do not add it here.

### 3c. Coverage guard

A test in `conformance.rs`, `every_api_name_is_exercised`, that:

1. evaluates, in the shared `JsEnv`, a module exporting
   `[...Object.keys(odm), ...prototype method names of odm.Solid,
   odm.Group, odm.Instance not starting with '_' and not 'constructor']`
   (use `odm_js::extract_export`, the way the runner reads `checks`);
2. concatenates every `.js` under `tests/conformance/unstable/`;
3. asserts each name occurs as `.name(` or `odm.name(` or `new odm.Name(`,
   with a small explicit allowlist (`Solid`, `Group`, `Instance`, which
   are exercised via `instanceof` and construction, and `children`).

Reading the names from the live surface, not a hardcoded list, is what
makes a newly added API function fail the gate until it has a test.

### 3d. Docs as tests

Four API pages have zero runnable blocks: `colors.md`, `determinism.md`,
`errors.md`, `three.md`. Give each at least one:

- `colors.md`: a group color inherited by an uncolored child next to an
  overriding sibling, and the opacity-multiplies claim.
- `determinism.md`: `Date.now()` called twice returns the same value;
  `Math.random()` returns a number (no pinning of the sequence).
- `errors.md`: two ` ```js error="…"` blocks, e.g. `union` with a Group
  operand (`must be Solids`) and a top-level `odm.box(1)` (`ops
  unavailable`).
- `three.md`: the torus generator converted and `rotateX(odm.deg(90))`.

### 3e. Later, not now

If the suite grows slow enough to notice (>1s), split `unstable_suite`
into one `#[test]` per file. Not yet.

## Phase 4: UI, cheap wins only

`odm-viewer-core` drives 48 tests through a headless `egui::Context`
(`inputs.rs` `Harness` is the pattern). Extend only where it is nearly
free and guards a user-visible property:

1. **Pure refusal logic, tested directly** (no egui):
   - `NewDialog::resolve` (`crates/odm-engine/src/viewer/new.rs`): an
     existing project folder is refused; a name containing `/` or
     starting with `.` is refused; empty name with a nameless folder is
     refused; a plain name in a plain folder resolves to
     `(path, name)`.
   - `check_destination` (`crates/odm-export/src/lib.rs`): a project root
     is refused; an unmarked dir inside the project that holds a `.js` is
     refused; a dir carrying `EXPORT_MARKER` (a previous export) is
     accepted; a fresh dir is accepted; a `.js`-holding dir *outside* any
     project is accepted. These protect users from hiding their own
     sources.
2. **`viewer/tabs.rs`**: `save` then `load` round-trips path, args,
   cascade and camera for two tabs and the active index; a corrupt or
   absent `.odm/viewer.json` yields `None`; an out-of-range active index
   is clamped.
3. **Headless dialog drive, only if each stays under ~60 lines** using the
   `Harness` pattern: `Picker` select → confirm yields `Outcome::Pick`,
   Escape yields `Cancelled`. Skip `ExportDialog` (needs a `JsEnv` and a
   thread) and `OpenDialog` browsing (filesystem-shaped, low value).

**Not doing**: `ViewerApp` tests (needs an eframe `CreationContext` with
wgpu state: a refactor for little return), any layout or placement
assertion, screenshot suites. `egui_kittest` stays the noted option if
the shell grows; looking at the viewer is the gui-testing skill's job.

## Phase 5: web export, guard the drift, keep the cost out of the gate

`crates/odm-web` is `cfg(target_arch = "wasm32")` throughout, so the gate
cannot reach it by construction; that stays. The JS half is covered by
`node_bundle.rs` (real bundle, real `runtime.js`, mock wasm module).

Two cheap native guards for the drift risks `notes/web-export.md` names:

1. **Ops parity** (`crates/odm-export/src/bundle.rs` tests): the set of
   `fn op_*` names in `crates/odm-js/src/ops.rs` equals the set in
   `crates/odm-web/src/executor.rs`. Read both source files via
   `CARGO_MANIFEST_DIR` and a regex; 17 names each today.
2. **Version-table parity**: make `odm_js::snapshot::{version_manifest,
   resolve_bare}` `pub`, and assert for every `ApiVersion` in
   `SUPPORTED` that `bundle.rs`'s counterparts succeed and agree on the
   file name of the manifest and of each bare specifier (`odm`, `three`).

Then one opt-in lane, as an `#[ignore]`d test
`crates/odm-export/tests/web_lane.rs` (`cargo test -p odm-export --test
web_lane -- --ignored`), plus `cargo xtask test-web` that runs
`build-web-template` and then that test. The test fails with a message
naming the xtask if the template is not built or chromium is missing.
Automates the recipe in `notes/web-export.md` on **both** lanes (WebGPU;
WebGL2 by injecting the `navigator.gpu` override into a copied
`index.html`). Asserts:

- the `ODM viewer: <lane>` console line names the expected lane
  (`--enable-logging=stderr`);
- no console line at error level;
- the screenshot is not a uniform color (GL failures are silently black);
- **geometry parity**: add a debug console line `ODM root: <hex>` where
  the web host publishes a build, and compare it to the root hash
  `odm_build` produces natively for the same view. This is the one
  property of the web lane that really matters; the rest is shared
  renderer code.

Never in the gate: the wasm toolchain build alone dwarfs the whole suite.

## Phase 6: hygiene

- `watcher.rs` is covered end to end by Phase 2 case 3; add unit tests
  only if that case proves flaky (debounce is 150ms of quiet).
- Keep the three `#[ignore]`d V8 abort reproducers in
  `crates/odm-js/tests/multi_snapshot.rs` as they are.
- Update `notes/architecture.md` "Testing" (new binaries, env vars, the
  e2e file, the opt-in web lane) and `tests/conformance/README.md` (new
  check kinds). Then delete this plan and fold its "what makes a test
  worth having" section into the architecture note.

## Explicitly not doing

- **CI**: one agent running the suite before committing is the gate.
- **Pixel goldens and hash goldens.** Renders differ across drivers,
  Manifold upgrades change triangulation, and a golden's failure never
  points at a fix. The analytic render assertions (blend math, wire width
  in px, depth ordering, alpha, determinism across runs) are the right
  level.
- **UI placement/layout tests**, exact CLI or error wording beyond the
  drift-tested `docs/cli.md`, and internal data shapes we expect to
  redesign.
- **Memoization observed from JS** (see 3b).

## Gotchas for whoever executes this

- One `JsEnv` per test binary, created before any build runs (`OnceLock`;
  `state::tests::env()` in odm-engine, `env()` in the odm-build suite).
  Creating a snapshot while another thread runs JS aborts the process.
- Every test binary that links the engine stack costs a large link: add
  odm-build tests as modules of `tests/suite/main.rs`, not new files.
  Crates with `[lib] test = false` need it flipped if you add unit tests
  to their `src/` (the manifests say so).
- Conformance `checks` live in the tested module, so a module that fails
  to evaluate cannot be an `error` check; use doctests for those.
- The runner's `area` check ignores instance transforms; JS `area()` does
  not. `t` defaults to 0. `set` values split into args/cascade by the
  root's `meta`.
- Never `CARGO_INCREMENTAL`, always `--workspace` (notes/build-environment.md).
- Socket paths: keep e2e temp dirs short and under `/tmp`.

## Order and cost

Phase 0 and 1 (an hour) → Phase 2 (about a day; the biggest gap) →
Phase 3 (the sustained work; commit per file or two) → Phase 4 (half a
day) → Phase 5 (guards: minutes; the lane: an afternoon) → Phase 6.
Expected end state: `cargo test --workspace` around 5s, with nothing in
it that can pass without running.
