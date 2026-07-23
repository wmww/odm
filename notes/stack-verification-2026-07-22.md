# Stack claim verification (2026-07-22, web-checked during mvp.md review)

Three research agents re-verified the load-bearing ecosystem claims behind plans/mvp.md.
Everything held EXCEPT the isolate threading model (see design-considerations.md, risk #1
— corrected there). Facts worth keeping:

## Manifold / manifold-csg — all confirmed
- Upstream v3.5.2 (2026-06-27), Apache-2.0, active. Determinism is ALWAYS-ON since
  v3.5.0 (cross-arch f64, CI-checked) — no flag needed, good for hashing.
- ExecutionContext (v3.5.0, PR #1704): cooperative cancel + progress, checked at
  boolean boundaries (not mid-op). `manifold-csg` 0.3.3 wraps it fully
  (`execution` module: new/cancel/is_cancelled/progress; `Manifold::with_context`;
  Send+Sync, Arc-shareable). Binding author is the same person who wrote the upstream
  ExecutionContext and ray-cast PRs.
- `manifold-csg` pins upstream v3.5.1; `manifold3d` is now a facade re-exporting it.
  API still moving (0.1→0.3 had migrations) — pin.
- **Build gotcha**: manifold-csg-sys `git clone`s upstream at build time → network
  needed for fresh builds; `MANIFOLD_CSG_LIB_DIR` is the prebuilt escape hatch.
  Affects "fresh checkout builds" acceptance + sandboxed/offline CI.
- Ray cast (`Manifold::ray_cast`), hull, simplify, CrossSection extrude/revolve all
  present in bindings.
- `MeshGL::merge` fixes only *slightly* non-manifold input (tolerance vertex weld) —
  NOT general repair. three.js generator output (duplicated verts) must be welded;
  open surfaces (Lathe/open Shape) may be unrepairable → agent-facing error path
  matters (spike 0c measures this).

## deno_core / rusty_v8 — confirmed except threading
- v8 crate 150.2.0 (2026-07-16, stable-declared, ~monthly major tracking V8);
  deno_core 0.408.0 (near-weekly 0.x breaking releases — pin hard).
- Snapshots: `create_snapshot` + `JsRuntimeForSnapshot`; ESM extensions included via
  `esm`/`esm_entry_point`. No DOM needed for module loading; three math/generators
  are DOM-free. Footgun: an extension with `esm` files must be IN the snapshot the
  runtime loads (deno#18979). No published spawn-latency numbers — spike 0b must
  measure.
- Isolate memory ~1–3 MB baseline (Cloudflare-scale precedent), but snapshot heap is
  deserialized PER isolate — budget snapshot-heap × isolate count.
- **Threading (REFUTED claim)**: `v8::OwnedIsolate` and `JsRuntime` are `!Send` —
  entered at construction, exited at drop, never movable across threads. rusty_v8
  removed `v8::Locker` (PR #272); reintroduction issue #643 still open. A worker
  CANNOT enter another doohickey's isolate on its own thread. Since V8 11.6 all
  runtimes need `JsRuntime::init_platform()` on a common parent thread first.
- Cross-thread cancel: `IsolateHandle` (Send+Sync) → `terminate_execution` works
  cross-thread. Open gotcha rusty_v8 #830: terminating during ES module evaluation
  can crash V8 — doohickeys are modules, so guard that window.
- Determinism: `--random-seed`/`--predictable` are PROCESS-GLOBAL V8 flags
  (`--predictable` also kills V8 background threads). Better: seeded-PRNG
  `Math.random` override + `Date` freeze in snapshot-time JS (per-isolate, survives
  snapshot). `performance` doesn't exist in bare deno_core (comes from deno_web,
  which we don't include).

## egui / wgpu / CI — all confirmed
- egui/eframe 0.35.0 (2026-06-25), wgpu 30.0.0 (2026-07-01). egui pins a wgpu major
  → upgrades come coupled. Cadence: egui ~3/yr, wgpu quarterly.
- egui 0.35 inspection protocol + `egui_mcp` (rerun-io/kittest_inspector, weeks old):
  agent can drive/read egui UI via AccessKit. egui_kittest = official snapshot-test
  harness (offscreen wgpu, per-OS thresholds, UPDATE_SNAPSHOTS).
- Custom wgpu in eframe: `egui-wgpu` `CallbackTrait` is the supported path; Rerun
  ships exactly this (eframe + re_renderer via callbacks).
- Offscreen render→PNG with same code path: first-class wgpu pattern; egui_kittest
  and Rerun snapshot-test real UIs offscreen in CI.
- **Pixel-identical is per-adapter only**: same device+code → same pixels; real-GPU
  vs lavapipe or across Mesa versions differs. Rerun CI: lavapipe on ALL platforms,
  Mesa pinned (25.2.7, reusing gfx-rs/ci-build binaries). Consequence: golden PNGs
  are lavapipe-only artifacts with pinned Mesa + small tolerance; local GPU runs
  can't diff against them. wgpu emits a "bad software rasterizer" warning under
  lavapipe — cosmetic, allowlist it.
- egui_tiles 0.16.0 and egui_dock 0.20.1 both current with 0.35.
