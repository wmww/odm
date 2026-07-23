# Ecosystem research (July 2026)

Web-researched 2026-07-22 via three parallel agents. Facts below were checked against
registries (crates.io, npm) and primary repos, not blog posts.

## Geometry kernels

- **Manifold** (elalish/manifold, Apache-2.0) — mesh boolean/CSG. Mature, very active
  (v3.5.2 June 2026). Guaranteed-manifold booleans, deterministic f64 results (v3.5.0,
  good for hashing/memoization), Minkowski sum/diff, SDF LevelSet meshing, convex hull,
  simplification, 2D cross-sections + extrude/revolve, ray casting. Adopted by OpenSCAD
  (default backend), Blender booleans, Godot, trimesh. Rust bindings consolidated 2026:
  `manifold-csg` (safe, over C API, tracks upstream closely; `manifold3d` is now a facade
  re-exporting it). C++ build dep underneath. No B-rep/STEP/exact fillets — mesh only.
- **truck** (ricosjp) — only established pure-Rust B-rep/NURBS kernel. Alive (pushed July
  2026) but bus factor 1 (ytanimura, ~90% of commits); crates.io releases stale since
  2024-09 (must use git). Booleans fragile (issue #68: panics at some tolerance values,
  runtime wildly tolerance-dependent). Fillets only in unreleased CHANGELOG ("single
  edge"). STEP partial via truck-stepio. Main known user (CADmium) archived 2025.
- **monstertruck** (virtualritz fork of truck) — claims Result-based booleans, real fillet
  engine (variable radius, chamfers), rewritten tessellation, STEP assemblies. New
  (crates.io June 2026), single maintainer, developed in a private repo synced to public,
  unverified claims. Watchlist item.
- **Fornjot** — DEAD. Archived 2026-06-19; README: "goals were never reached".
- **OpenCascade (OCCT)** — only open-source kernel with mature fillets/chamfers/robust
  booleans/full STEP. OCCT 7.9.3 (Dec 2025). Rust story weak: opencascade-rs is active
  but spare-time single-maintainer, LGPL, vendored C++ build, crates.io stale (use git);
  anvil stalled; a new binding effort announced Apr 2026 is too early to judge.
- **fidget** (mkeeter) — implicit/SDF kernel, pure Rust, active, JIT evaluation, octree
  meshing. Great for f-rep/organic; not B-rep.
- **csgrs** — pure-Rust BSP-tree CSG; active but float-BSP booleans are inherently less
  robust than Manifold (coplanar/degenerate cases). **boolmesh** — young pure-Rust
  reimplementation of Manifold's algorithm; watchlist.
- **Zoo/KittyCAD** kernel is proprietary cloud — not usable locally.

## Native Rust viewer stacks

- **Bevy 0.19** (June 2026) — best out-of-box PBR of Rust options; 0.19 upstreamed the
  CAD-viewport furniture: interactive transform gizmos, InfiniteGridPlugin, picking
  (since 0.15), entity-inspector components. New BSN scene system. UI: bevy_feathers
  still experimental-flagged; bevy_egui solid. Headless render-to-PNG: official
  `headless_renderer` example, works, ~200 lines of plumbing that breaks across versions
  (0.19 replaced render graph with ECS systems). Churn is the highest of all options:
  breaking release ~3x/year with real migration guides. Compile times still a top
  complaint. Can be embedded as a library (custom runner) but the grain is "Bevy owns
  the app".
- **egui 0.35 + wgpu (Rerun pattern)** — egui funded by Rerun; 0.34 (Mar 2026) was a big
  polish release (skrifa hinted fonts, harfbuzz kerning, styling classes, Panel API
  redesign); 0.35 added an inspection protocol + `egui_mcp` crate (agent drives/reads UI
  via AccessKit tree — directly relevant to ODM). Docking healthy: egui_dock (active) and
  egui_tiles (Rerun's). Rerun 0.34 is the product-scale existence proof: eframe shell +
  re_renderer (their own wgpu renderer). re_renderer is published and usable standalone
  but versioned in lockstep with Rerun, no stability promises — mine for patterns rather
  than depend. Headless: best by construction (own wgpu pass → offscreen texture →
  PNG, identical to viewport path). Churn: ~2x/year mechanical egui migrations +
  quarterly wgpu major bumps.
- **Iced 0.14** (Dec 2025, after 15-month gap) — Shader widget for custom wgpu viewport
  works; headless UI testing new in 0.14. But no docking system, young table widget, DIY
  trees, ~1 lead maintainer, slow cadence. COSMIC ships on it but maintains libcosmic.
- **Godot 4.6 + gdext 0.5** — best out-of-box editor-style UI (docking/trees/inspectors).
  4.6 shipped LibGodot (host process can own the loop) but gdext has NO LibGodot support
  yet (issue #1488 open, unanswered). Headless is the worst of all options: `--headless`
  disables rendering entirely; `--offscreen` is a draft PR, Windows/GL-only. V8-in-Godot
  proven (GodotJS) but means Rust↔Godot↔V8 FFI layering.
- **three-d 0.19** — lightweight renderer with PBR + real headless example, but
  OpenGL/WebGL2 (not wgpu), solo maintainer. Cheap viewport option, wrong long-term bet.
- **GPUI** (Zed) — now on crates.io, gpui-component has dock/table/tree; high polish
  ceiling but custom-wgpu-viewport integration is not a supported path. Watchlist.

## Web-viewer route (Three.js renderer of record)

- **Three.js r185** (July 2026) — healthy, ~6-week cadence, still 0.x with rolling API
  breakage (10-release deprecation window). WebGPURenderer is the strategic renderer
  (WebGL2 fallback built in) but has real perf regressions on many-mesh scenes and
  ongoing TSL API churn; r184/r185 changed blending/premultiplied-alpha behavior —
  upgrades can change pixels. Pin hard if render stability matters.
- **Headless three.js**: default answer is still puppeteer + headless Chrome
  (SwiftShader or GPU-with-flags). Three.js's own CI does this and does NOT achieve
  pixel-identical (0.1/pixel + 0.1% tolerance, ~60-example exception list, WebGPU
  device-loss auto-restart). Chrome is a ~200-500MB-RSS dependency; warm instance gives
  tens-of-ms renders, cold ~5s. Emerging no-browser path: Dawn-in-Node (`webgpu` npm,
  official dawn/node) and young wrappers (headless-three-webgpu, 6 stars) — proof of
  existence, not infrastructure.
- **Tauri 2.10** — fine as a shell, but no offscreen/headless webview rendering (wry#391
  open for years) and uses per-OS system webviews (WebKitGTK on Linux) — different
  engine/pixels per platform. Rules it out where pixel parity matters.
- **JS embedding**: rusty_v8 stabilized (tracks V8, v150 July 2026, Deno-maintained);
  deno_core 0.408 active — JsRuntime + snapshots (snapshot framework+three-math for fast
  isolate startup). rquickjs fine but interpreter-only (5-30x slower on math-heavy
  code); Boa slower still. V8/deno_core is the right call.
- **Precedents**: nobody does "native engine + browser three.js viewer + identical native
  headless renders". Zoo: engine-side GPU rendering, WebRTC video stream to client,
  agent PNGs from same engine framebuffer. OnShape: cloud kernel tessellates, client
  renders WebGL. chili3d: everything in-browser (OCCT wasm + three.js). Plasticity:
  Electron + three.js + native kernel module. Everyone picks ONE renderer of record.
