# Testing plan

From the 2026-09-09 test survey. Current state: 322 tests, 0 failures,
~2.5s after compile; no CI (deliberate — `cargo test --workspace` before a
commit is the gate). This plan fixes what is vacuous, fills the gaps that
matter, and says what we are deliberately not testing.

Standing principles for this repo's tests:

- Test **properties that must hold**, not the current shape of things we
  expect to redesign. A test that has to be edited every time we improve
  wording, layout, or a data structure is a tax, not a guard.
- The one gate is `cargo test --workspace`, and it stays **fast**
  (budget: under ~10s). Anything needing a browser, a wasm toolchain, or a
  compositor goes behind an explicit `cargo xtask` command.
- A test that cannot run must **fail**, not silently pass.

## Phase 1 — no vacuous passes (small, do first)

`odm-render/tests/render.rs::renderer_or_skip()` returns `None` and the
test passes when no GPU adapter exists. On a machine without Vulkan the
whole 33-test render suite goes green while testing nothing.

- Make adapter acquisition **panic** by default, with the message naming
  the escape hatch.
- Escape hatch: `ODM_TEST_NO_GPU=1` skips the render tests explicitly, for
  a machine that genuinely has no adapter.
- Print the adapter name once per run (`eprintln!`), so a silent fallback
  to a software rasterizer is visible rather than inferred.
- Same treatment for the `node`-missing skip in
  `odm-export/tests/node_bundle.rs`: fail unless `ODM_TEST_NO_NODE=1`.
  node is a real dependency of that lane, not an optional extra.

## Phase 2 — end-to-end integration (best coverage per unit of work)

Nothing today spawns the `odm` binary. That leaves untested: arg dispatch
(`crates/odm/src/main.rs`, 175 LOC), engine startup/shutdown, the socket
lifecycle, the inotify watcher (`watcher.rs`, zero tests), and the whole
CLI↔engine round trip.

It is cheap. Measured on this machine (debug build, examples/piston):
engine socket up **86ms** after `odm run --headless`, `status` **8ms**,
`render` to PNG **61ms**, post-edit `inspect` **19ms**. A whole e2e file
with a handful of engine spawns lands well under 1s.

New `crates/odm/tests/e2e.rs` (the `odm` package is bin-only, so this test
binary links nothing heavy — it just spawns `env!("CARGO_BIN_EXE_odm")`).
Helpers: copy an example to a **short** temp path (`/tmp/<short>`, not the
scratchpad — `SUN_LEN` limits the socket path), spawn the engine, wait for
the socket, kill on drop. Share one engine across the read-only cases;
spawn fresh ones only for lifecycle cases.

Properties to assert:

1. **Startup**: socket appears, `status` names the project and reports the
   default view built ok.
2. **The hot-reload invariant** — the headline one: edit `root.js` on
   disk, then the *next* query reflects it with no explicit sync and no
   sleep. Repeat for a nested part file, and for a rename. This is the
   only place the watcher + sync-on-query path is exercised as shipped.
3. **Failure and recovery**: break a file → the query returns the JS error
   message; renders still serve the last good scene; heal it → the next
   query is clean again.
4. **Render**: PNG exists at the requested size, is not a uniform colour,
   and two identical requests are byte-identical.
5. **Chat round trip**: `say` from one process, `poll` from another
   returns it, `ack` retires it, a second `poll` does not repeat it.
6. **CLI errors an agent will actually hit**: no engine running (message
   names the `odm run` command to fix it, non-zero exit); a second `run`
   on the same project refuses; unknown command; malformed JSON.
7. **Shutdown**: the engine removes its socket, so a restart works.
8. **`odm docs` smoke**: every topic the index lists renders and exits 0.
   This is the agent's documentation surface and it has zero tests today.

Assert on *behaviour and structure* (exit codes, JSON fields, file
existence), never on exact prose — `docs/cli.md` drift is already guarded
by `cli_reference_is_current`.

## Phase 3 — the JS API (the main body of work)

The API is the product surface; it should be the best-tested thing here.
Two existing guards stay: the conformance suite
(`tests/conformance/unstable/`, 15 projects) and the docs doctests (~27
`js` blocks). The runner already aggregates every failure per run and
costs ~10ms per project, so there is room to grow it several times over.

Holes found in the survey:

- **`hull()` and `label` have no conformance test at all** (docs only).
- Thin — one or two files each: `revolve`, `extrude`, `clearance`,
  `opacity`, `sphere`, `area`, `rotate`, `about`.
- The vendored three.js subset is covered only incidentally
  (`three-f64.js`, `fromThreeGeometry` twice). No test of the math types
  the API actually consumes.
- `docs/api/determinism.md`, `errors.md`, `colors.md`, `three.md` have
  **zero** runnable `js` blocks, so those pages are not drift-checked.

Work:

1. **Cover every export.** One conformance file per API area, each
   asserting derivable values (analytic or exact CSG arithmetic — never a
   number pasted from what the engine printed; the README already says
   so). Fill `hull`, `label`, and thicken the thin ones.
2. **Interactions, not just functions** — this is where the real bugs
   live, and where a per-function checklist would miss them:
   - transform ∘ CSG order (`a.translate(…).subtract(b)` vs
     `a.subtract(b.translate(…))`), and transforms composing with
     `about`;
   - non-uniform scale through booleans and through `raycast`/`clearance`
     (the area caveat lives here);
   - nesting: groups in groups, instances of instances, a shared solid
     used at two transforms (geometry interned once — `examples.rs`
     asserts this for the assembly; the API should assert it too);
   - colour/opacity/label inheritance and override down a group tree;
   - `invoke` × cascade × memoization as seen from JS: a child reading `t`
     rebuilds, a sibling that does not read it does not;
   - degenerate and boundary inputs: empty group, zero-size box, a
     subtract that empties a solid, a hull of one operand, a raycast that
     misses, a clearance of touching solids.
3. **three.js subset**: one file per consumed type (Vector3, Matrix4,
   Euler, Quaternion, the generators feeding `fromThreeGeometry`),
   asserting the values the ODM API derives from them — not three's own
   semantics.
4. **Coverage guard**: a test that reads the exports of
   `framework/odm/index.js` and asserts each name appears in at least one
   conformance file, with a small explicit allowlist for anything
   deliberately uncovered. Mechanical, but it is the thing that keeps
   "every function is tested" true as the API grows.
5. **Docs**: give the four example-less API pages at least one runnable
   `js` block each, so their claims are compiled.
6. If the suite ever gets slow enough to notice, split `unstable_suite`
   into a test per file for isolation and parallelism. Not yet.

## Phase 4 — UI (cheap wins only)

`odm-viewer-core` already drives 48 tests through a headless
`egui::Context` with real hit-testing — inputs (21), tree (8), scroll (8),
camera (4), theme (5). That pattern works; extend it exactly where it is
free.

The four dialogs and the picker (`odm-engine/src/viewer/{open,new,export,
pick}.rs`) each take only `&egui::Context` and return an `Outcome` — they
are drivable headlessly today, no refactor needed:

- browse → select → confirm produces the right `Outcome`;
- cancel/escape produces none and leaves no side effect;
- the refusals that protect the user: export into a project root or an
  unmarked dir with `.js` files, New over existing files.

Also `viewer/tabs.rs::load`/`save` — pure file I/O, a round-trip test plus
"a corrupt/absent tab file degrades to a sensible default".

**Not doing**: `ViewerApp` itself needs an eframe `CreationContext` with a
wgpu render state, so full-shell tests would mean a real refactor for
little return. No screenshot/snapshot suite — `egui_kittest` stays the
noted option if the shell grows, and looking at the viewer stays the
gui-testing skill's job.

## Phase 5 — web export (guard the drift, keep the cost out of the gate)

`crates/odm-web` (966 LOC) is `cfg(target_arch = "wasm32")` throughout, so
`cargo test --workspace` cannot reach it by construction — that is a
deliberate design choice (native builds see an empty stub) and it stays.
The JS half is already well covered by `node_bundle.rs`, which runs the
real bundle and the real `runtime.js` in node against a mock wasm module.

Two cheap native guards for the drift risks `notes/web-export.md` names,
both pure source comparisons, both microseconds:

1. **Ops parity**: the `op_*` sets in `odm-js/src/ops.rs` and
   `odm-web/src/executor.rs` must match. They do today (17 each, same
   names); the note says to keep them in sync by hand. Make it a test.
2. **Version-table parity**: `version_manifest`/`resolve_bare` in
   `odm-js/src/snapshot.rs` and `odm-export/src/bundle.rs` must agree on
   every `ApiVersion`.

Then one opt-in lane, `cargo xtask test-web`, automating the manual recipe
in `notes/web-export.md` (build template → export an example → serve →
headless chromium on **both** lanes; chromium, node and a real GPU are all
present on this machine). Assert:

- the `ODM viewer: <lane>` console line names the expected lane;
- no JS errors on the console;
- the screenshot is not blank/uniform (GL failures are silently black);
- **geometry parity**: the page reports its published root hash, and it
  equals the native build's for the same view. This needs a small debug
  hook in the host (console line, or a query param). It is the one
  property of the web lane that really matters — everything else is the
  renderer, and the renderer is shared code.

Never in `cargo test`: the wasm toolchain build alone dwarfs the whole
suite.

## Phase 6 — hygiene

- `watcher.rs` has no unit tests. Phase 2 covers it end to end, which is
  the coverage that matters; add unit tests only for debounce/rename
  edge cases if e2e turns up flakiness.
- Keep the three `#[ignore]`d V8 abort reproducers as they are — they
  abort the process by design and are correctly documented.

## Explicitly not doing

- **CI** — the current workflow is one agent running the suite before
  committing; that is the gate.
- **Pixel goldens.** Renders differ across drivers, and Manifold upgrades
  legitimately change triangulation; goldens would be a rolling
  maintenance cost that catches little. The existing analytic assertions
  (blend math, wire width in px, depth ordering, alpha, determinism
  across runs) are the right level.
- **Exhaustive widget/state tests for UI that is still moving**, exact
  CLI/error wording beyond the drift-tested `docs/cli.md`, and internal
  data shapes we expect to redesign.

## Order and cost

Phase 1 (tiny) → Phase 2 (~1 day, biggest gap closed) → Phase 3 (the
sustained work) → Phase 4 (half a day) → Phase 5 (guards are minutes, the
xtask lane is an afternoon) → Phase 6 (as needed). Expected end state:
`cargo test --workspace` around 4-5s, with nothing in it that can pass
without actually running.
