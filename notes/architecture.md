# Architecture (as built)

The system as it exists (MVP completed 2026-07-22). Why it's this way:
`design-decisions.md`; measured facts behind the design: `spike-findings.md`.

## Project format

- A project is a directory marked by `odm.toml` (`name` + `engine`, unknown
  keys rejected; NOT part of generation identity). Every `*.js` under it
  (recursive, skipping dot-dirs like `.odm`/`.git` and `node_modules`) is a
  doohickey, identified by project-relative path. Any file is viewable;
  `root.js` is pure convention — the default target when a query names no
  path.
- **Views**: a view = (path, args, cascade), evaluated against the current
  generation. Queries and viewer tabs each hold one. `t`/animation is not a
  feature — just a ranged cascade input the viewer gives a transport.
- **Inputs** (`docs/api/inputs.md` is the contract): `export const meta =
  { inputs, presets }`; one map, name → profiled JSON Schema + ODM keys
  (`cascade`). Read via `ctx.input(name)`; plain inputs come from the caller's
  args (defaults merged into memo identity), cascade inputs resolve up the
  invoke chain — nearest provided value wins, view outermost, declarations
  auto-provide their defaults for their subtree. `ctx.invoke(path, args?,
  cascade?)`.
  Extension types (solid/vector2/vector3/quaternion/matrix4/color) are
  canonical JSON on the wire, hydrated to THREE instances by ctx.input.
  Schemas are recursive (2026-08-17, was plans/structured-inputs.md):
  ext types at any depth (solid top-level-only), nested `default`s
  *applied* — an absent object property with a default is filled at
  normalization; an array `items.default` is the panel's new-element
  template (`meta::synthesize`) — plus `variants`/`tag` tagged unions
  (internally tagged wire form, if/then desugar, own error for a bad
  tag) and `additionalProperties` maps. `ctx.input` hydrates by walking
  the declared schema (decls carry the whole schema). Cascade values
  are not normalized into the env (memo identity keeps the raw form);
  the JS-side hydrate walk fills their nested defaults so both channels
  read the same.
  Validation at every boundary via the `jsonschema` crate; unknown
  args/schema keys/input names are errors, and nested value errors name
  the path (`at /0/position`).
- The `//!` comment block doubles as prose description (summary line +
  body), parsed at sync time without evaluating the module.
- `.odm/` is engine-owned (socket `.odm/engine.sock`, `renders/`,
  `viewer.json` tab persistence). Excluded from generation hashing.
- **Project resolution** (`odm-cli`, reused by `run`): an explicitly named
  dir — `odm run <dir>`, `--project <dir>` — is taken exactly as given and
  must hold `odm.toml` (`project_dir`); **nothing ever walks up from a path
  someone named**. Only the cwd fallback walks: client commands take the
  nearest project at or above cwd, git-style, and talk to *its* socket
  (`find_project`). Marker, not socket: a subdirectory of an unserved
  project then says "no engine, start one" instead of reaching past it to
  whichever project further up happens to be running. `run` does not walk
  at all — it serves the dir named or cwd, which must be a project; the one
  exception is the viewer with no dir, which asks (Open Project screen,
  below) rather than failing.
  `is_project` is only the marker's presence, not its contents: a malformed
  `odm.toml` is scan's complaint to make, and refusing to open it would
  leave no way to fix it in the viewer. Two copies of that one-liner
  (`odm_build::is_project`, `odm_cli::is_project`) — odm-cli stays
  dependency-light on purpose.
- `odm.toml` is one of the two project files the engine writes: it records its
  `engine` version back on open (warning first if the file names a newer
  engine). The other is the agent files — see below.
- **Agent files** (`AGENTS.md`, `CLAUDE.md` at the project root): the standard
  prompt lives between `<!--- BEGIN/END STANDARD ODM PROMPT --->` lines, and a
  well-formed pair is the file's opt-in — `odm_prompt::sync` splices the
  current prompt in on every project open (viewer *and* headless), touching
  nothing outside the markers and not writing at all when the block is already
  current. A new project (File ▸ New Project) gets AGENTS.md with the block and
  CLAUDE.md as a relative symlink to it (`odm_prompt::create` — skipped
  entirely if either name is already taken, leaving the open-time question to
  offer the block). Anything else is a viewer question,
  one per logical file (the symlink pair dedupes by canonical path, asked about
  as AGENTS.md): an existing unmarked file offers an append, and no agent file
  at all offers to create the pair. Answers are not recorded — a "no" is asked
  again next open — and headless never asks. Quiet cases (only one file, a
  regular CLAUDE.md, a symlink out of the project, a dangling one) are left as
  the user set them up; a lone marker warns and is never guessed at.

## Crates

- `odm-ir` — Mesh/Node/Scene/Color/Transform + canonical bit-exact blake3
  hashing (FORMAT_VERSION in canon.rs; bump on any encoding change — floats
  hash as raw IEEE bits, no -0.0/NaN canonicalization, equal hash ⇒
  byte-equal), canonical JSON hashing (sorted keys, f64 numbers).
  `Node.children` are content hashes, not inline nodes: every node is
  interned in the store, so a repeated subtree (4 wheel placements, a 20×20
  grid) is stored and hashed once and a root hash costs O(root's own fields),
  not O(whole tree). `Node::refs` = mesh + children; walking a tree means
  `store.get` per child.
- `odm-store` — content-addressed objects, generations (live until
  `release_generation`; one is live at a time — the build engine's current),
  mark-sweep GC (roots = generation roots + memo outputs; quiescence-only;
  `Object::refs` walks the node graph as well as meshes, so a live root pins
  its whole subtree),
  memo cache (key = code+args hashes; per key a bounded MRU list of
  entries — one per environment seen, `MEMO_PER_KEY` = 64 — so scrubbing
  `t` revalidates instead of rebuilding; global LRU cap `MEMO_CAP` = 4096
  entries since every entry's output is a GC root; entry = recorded deps +
  output; `Dep::Invoke` stores the actual args Value so validation can
  re-run invokes, `Dep::Cascade` a value hash — missing keys hash a
  sentinel, use `odm_js::cascade_value_hash`).
- `odm-kernel` — manifold-csg wrapper: primitives (cylinder along Z),
  extrude/revolve (around Z), sweep (= extrude with one slice per station,
  then `Manifold::warp` onto JS-computed affine frames; the kernel never
  sees a path, so caps/holes/fill/welding are the extrude path's),
  booleans/hull with per-operand transforms,
  weld with boundary-edge diagnosis (Manifold's own error is bare
  NotManifold), raycast (Manifold returns distance as a *fraction* of the
  segment; kernel converts), clearance (signed distance via the dist.rs
  triangle BVH; see notes/agent-surface.md), volume/area/bounds, CancelToken
  (ExecutionContext), Hash→Manifold cache with rebuild-from-store fallback.
  Segments are always explicit — kernel rejects <3; framework defaults:
  cylinder 64, sphere 48, revolve 64.
  Mesh positions are f64 end to end (MeshGL64 both directions; the three
  generators emit Float64BufferAttribute positions and op_solid_from_mesh
  takes Float64Array): the store boundary never quantizes, so rebuilds
  re-weld exactly and baked far-from-origin transforms keep detail. The
  ONE f32 conversion is per-mesh GPU vertex upload in odm-render/gpu.rs.
  `precision` test files in odm-ir/odm-kernel/odm-render plus
  tests/conformance/unstable/three-f64.js are f32-regression tripwires
  (bit-exact 0.1 / near-1e7 probes) — an `as f32` sneaking into any seam
  fails one of them; keep new position paths covered there.
- `odm-js` — the *native executor*: deno_core =0.408.0; per-build
  disposable isolates from ONE
  snapshot embedding `framework/` (odm API + three r185 subset, every
  supported API version's surface manifest — see "API versions" below); ops
  extension; dep recording; console capture (logs are always data —
  `run_build` errors are `FailedBuild` = error + that build's logs, never
  text-embedded; the scheduler folds them into pass logs); `run_build` is the single
  entry point (`extract_export` the side door for reading a module's export
  without building — the conformance runner uses it). `ir_json::node_from_json` interns the framework's IR JSON into
  the store and returns the root hash; `ctx.invoke` crosses the boundary as a
  hash string, and a JSON node `{"ref": "<hex>"}` (no other keys) *is* that
  stored subtree — so an Instance with no transform/color/name reuses the
  invoked subtree's hash outright. Isolates nest strictly LIFO per thread. Module URLs:
  framework at `file:///odm/framework/*` (bare 'three'/'odm' resolve there);
  doohickeys at `file:///odm/project/<path>` — single file, no project
  imports. op2 quirks: `op_invoke` must be `#[op2(reentrant)]` (nested build
  ops re-enter); no fixed-size-array params (use Vec<f64>);
  `serde_json::Value` must be written fully qualified. Module loading is
  driven by futures::executor::block_on (no tokio — nested block_on works).
  Since the executor seam (2026-08-17) odm-js *depends on odm-build* and
  implements its `Executor` trait for `JsEnv` (`run_build` +
  `extract_export`; isolate handles wrapped as `InterruptHandle`); the
  shared types (`BuildInput`/`Output`, `Invoker`, `FailedBuild`,
  `cascade_value_hash`, `node_from_json`, `ApiVersion` + pragma/doc
  parsing) live in odm-build's `executor`/`version`/`ir_json` modules.
- `odm-build` — one `BuildEngine` per project; V8-free (compiles for
  wasm32 — `BuildEngine::new` takes an `Arc<dyn Executor>`; the engine
  passes `Arc<JsEnv>`, the web export its JS-glue executor; timing and
  the cancel watchdog are cfg'd off wasm); `sync()` rescans it and
  reuses the current generation while the source hashes match (retiring the
  old one otherwise); pass = generation + view (path, args, cascade).
  Per-build environments: each invoke path derives its env (invoke cascade
  values overwrite, declaration defaults fill), hashed for the in-flight
  registry and cycle keys. `get_or_build` validates+merges args against the
  file's meta (effective args are the memo identity), eagerly validates and
  dep-records every declared cascade input (an unread declared input still
  keys memoization — consistency), then runs. Salsa-style validation and
  early cutoff; `Dep::Invoke` carries its cascade map. `meta.rs` = schema-
  profile allowlist walk + extension desugaring, cached by code hash
  (`BuildEngine::meta`, extraction via `extract_export`); `report.rs` =
  post-build input report walked from memo entries: ONE flat list of
  everything view-settable (target's plain inputs + fall-through cascade
  names, each entry carrying `kind` and the winning declaration's full
  `schema` — flat type/range/choices fields are derived methods; `to_json`
  prints the schema, so `odm inspect` shows element shapes), winning
  declarations, lints
  (conflicting defaults/types, unread cascade values, plain-shadows-cascade)
  — the input panel's data source and the input-name typo check
  (`check_input_names`); `declared_entries` is the failure-path subset
  (target's declared schema, no walk needed). Each pass also collects
  `BuildStats` (per-doohickey runs + self-time, memo hits) into
  `PassResult.stats`, surfaced by `"stats": true` on any view command.
  In-flight registry (wait-for-in-flight + wait-graph cycle detection);
  cancellation (token + TerminateExecution post-module-eval). Cycle check is
  keyed on (path, args-hash, env-hash) so bounded recursion works; memo
  entries carry console logs and replay them on hits. Entries store
  success-or-failure (`MemoOutput`): pure failures (`Js`, `BadOutput`)
  memoize with the deps recorded up to the throw and replay kind + message
  + logs byte-identically (error + logs are part of a build's output by
  construction); `Cancelled`/`Internal` (not values of the function) and
  `Cycle` (message embeds the chain) never memoize, and failure entries
  pin no GC output. Passes run
  concurrently across threads (each CLI connection + the build loop); the
  registry's cross-thread dedup/cycle machinery is what makes that safe
  (`concurrent_same_pass_dedups`, `commands_overlap_an_in_flight_build`).
  Within one pass `get_or_build` still recurses inline.
- `odm-render` — wgpu =29.0.4 (MUST track egui's pinned wgpu major).
  MUST stay WebGL2/downlevel-compatible: the web export falls back to
  wgpu's GL backend (constraint list at the top of odm-render/src/lib.rs;
  policy + both-lane test recipe in notes/web-export.md);
  the single flattener `flatten_node` (color replace-wins inheritance,
  multiplicative opacity product into instance alpha, sRGB→linear — the
  only conversion in the system, world AABB, node ids for picking — engine
  and viewer both use it) → one draw_indexed per
  instance with dynamic uniform offsets (not instanced draws), flat shading
  via screen-space derivatives, auto-framing perspective/ortho cameras;
  `render_png` and the viewer viewport share `render_to_target` (renderer
  owns every intermediate texture; callers hand one final single-sample
  view); the GPU mesh cache is pruned to the live scene after every
  render/publish. `Renderer::with_device` for the shared eframe device.
  **Pass structure** (2026-08-16 restructure; two categories only, no
  per-geometry depth/blend hacks): opaque pass (premultiplied linear
  Rgba16Float color cleared to premultiplied background + depth) → exact
  translucency via **depth peeling** (`peel_layers` front-to-back layers,
  default 4: geometry into a scratch layer with blend-replace + depth
  isolating the nearest not-yet-peeled fragment, discarding at-or-nearer
  than the prev peel depth or behind opaque; then a fullscreen
  under-composite into the accumulation; ping-pong peel depth textures) →
  unsorted tail pass for deeper fragments (blend under, draw order) →
  fullscreen compose (accum over opaque, un-premultiply) into the target —
  which makes transparent-background PNG alpha exact. Determinism rules:
  one peel pipeline per geometry family reused across layers, `@invariant`
  positions so peel and tail agree bit-exactly (tested: N=1 == N=4 canary);
  coplanar translucent fragments collapse to one layer by design (equal
  depth discards — no double-darkening, but stacked identical surfaces
  don't accumulate). Translucent mesh passes cull back faces: a 30% solid
  reads as one veil. Effective alpha = color.a × opacity product ×
  `RenderOptions.opacity` (x-ray; render request `opacity`, viewer
  View ▸ X-Ray at 0.3); partition ≥1 → opaque, <1 → peeled. **All lines
  are translucent**: wires and grid share one line-quad path (instance =
  endpoint pair, widened to per-slot pixel width + half-pixel AA feather;
  coverage alpha; grid minors also fade by projected line spacing —
  smoothstep 2..8 output px — so dense regions melt instead of moiréing);
  `pick_wire` does the matching screen-space selection. Wireframe mode =
  skip the fill passes. **No MSAA** (dropped deliberately — silhouettes
  stay aliased for the retro look; the artifacts that hurt were grid moiré
  and wireframe speckle, fixed by the AA/fade above, not by MSAA);
  `RenderOptions.supersample` (request `supersample`, default 1) renders k×
  larger internally (premultiplied compose → box downsample; internal size
  validated against the device texture cap) for AA on demand. Perf (debug
  build, 2026-08-16): 400-sphere full x-ray ≈ same ~80 ms CLI round-trip
  as opaque; 4× supersample ~120 ms — the N+1 translucent draws are
  nowhere near a bottleneck at CAD scale.
- `odm-viewer-core` — the viewer's *read side*, shared between hosts
  (2026-08-17 extraction; desktop today, the web export is the planned
  second host — plans/web-export.md). Holds `theme/` + `icons` + the
  bundled font/icon assets, the `Orbit` camera, `OffscreenTarget`, the
  scene tree, the input panel, the console pane, the `t` transport, `Tab`
  (per-tab state; persistence stays with the host), and `Viewer` — the
  shared per-tab machinery (poll published → flatten + tree snapshot,
  viewport paint, orbit/pan/zoom, solid/wire picking, frame, transport,
  the app-wide wireframe/x-ray/grid toggles). Viewport keys (gated off
  while a text field or host chrome holds the keyboard): F frame, X
  x-ray, W wireframe. Parameterized over the
  `Engine` trait: `set_view` (submit a view for a slot, latest-wins),
  `published` (the slot's last result — `Published` is defined here,
  state.rs re-exports it), `store`, `raycast`, `set_selection` (hosts
  that report selection onward). Hosts own tabs, the `Renderer` (desktop
  shares it with the activity view), layout/panels, and persistence;
  methods return whether the view changed so the host can save. Hard
  rule: no odm-js, no `EngineState`, no sockets/watcher — it compiles to
  wasm, and the web export (odm-web) is the second host.
- `odm-engine` — library, entered via `odm run` (`run_headless(project)` /
  `run_viewer(Option<project>)`).
  Headless runs the same background threads as a viewer session (build
  loop + watcher + health sweep, `session::spawn_background`) minus the
  UI — the engine keeps its slots' published values current, a viewer is
  just eyes on them; `status`/poll are truthful headless and the memo
  cache stays warm. Default: + eframe
  viewer (menu bar, tab strip — one view per tab, persisted in
  `.odm/viewer.json`; classic notebook tabs, each with its own close box, a
  red label when that tab's last build failed, and a magnifier at the end
  opening the doohickey picker. The *last* tab closes too (Ctrl+W, or its
  box): no tab is a real state — empty panels, a blank viewport well
  (`blank_viewport`), `set_active_slot(None)`, no view registered, chat
  messages with no view snapshot — and it persists as an empty tab list in
  viewer.json —, offscreen texture viewport via
  register_native_texture, orbit/pan/zoom, one **side bar** down the right
  holding the generated input panel above the scene tree with a draggable
  split between them (inputs from the tab's fall-through report: label/value
  blocks with check boxes, radios, sliders, number/JSON text fields,
  presets, and `t` among them with a play button; there is no left panel
  and no status band), one resizable bottom dock holding **Agent** and a devtools-style **Output**
  as two tabs
  (`theme::tab_strip`, the plain version of the view strip; one dock per
  window, showing the active tab's console; the Output label carries a lamp of
  its own — absent when the build said nothing, gray for logs, amber for a
  warning/logged error, red for a thrown one, so a failed build says
  so from the Agent tab; the Agent label carries a status lamp —
  `StripTab::lamp` — dark red when nothing is listening, green when an
  `odm poll` is waiting, blinking while the agent has a task)
  with last-good scene (`Published.logs` is latest-attempt: success or
  failure, colored by `LogLevel`; a failed build's error is the final
  red entry — presentation-only merge, `Published.error` stays its own
  field), click-select via CPU raycast when shaded / nearest-wire
  screen-space pick when wireframe, shift-click to select several,
  **agent activity view** — see below),
  background build loop over the active view slots
  (per-slot latest-wins, Pass::cancel on same-slot supersede; a new
  generation re-queues every slot), notify-based watcher (150ms
  debounce; its dot-dir filter applies to the path *relative to the project*,
  since the project itself may live under one). The viewer never polls: it
  repaints when `EngineState::wake` fires (set by the viewer, unset when
  headless), i.e. on every `published` change. It also owns its winit event
  loop so `SlowIdle` can fix up what eframe leaves behind — see viewer/idle.rs
  and "Owning the event loop" below.
  **Agent activity view** (2026-08-17, was plans/agent-activity-view.md): a
  render of the last agent CLI action, in its own resizable column down the
  right of the chat tab (was a faded backdrop behind the transcript until
  2026-08-17 — text over a render served neither).
  raycast/inspect/render handlers push `ActivityEvent`s (state.rs: capped
  deque ~8, gated on `viewer_attached()` = wake hook set, so headless pays
  nothing — `cmd_render` even skips the RGBA capture). Events are
  self-contained (flattened `Arc<RenderScene>` / RGBA pixels, never store
  hashes — memo entries are LRU-evicted, so display-time store reads could
  dangle). Viewer side (viewer/activity.rs): card queue snaps
  through at 1.2s DWELL, last card persists, pure `advance_cards` policy is
  unit-tested with injected time; raycast cards frame the ray side-on (yaw ⊥
  azimuth, pitch 0.5; near-vertical keeps default) and draw it via
  `RenderOptions.overlays` (`OverlaySeg` — generic odm-render wire-pipeline
  feature, depth-tested); inspect cards frame the node's AABB and brighten
  its instances with the selection formula; render cards upload RGBA as an
  egui texture, *cover*-cropped (`cover` returns the uv window — the panel is
  whatever shape the user drags, and letterboxing would waste it). Cards render once per (card, size) via the
  shared-device renderer into an odm-viewer-core `OffscreenTarget` (the
  ViewportTex machinery extracted for reuse — future render windows are one
  OffscreenTarget + camera + scene each; per-window Orbit not built yet).
  `ActivityView::panel_ui` owns the whole column: sunken face-coloured well
  (empty it reads as panel, not as a black hole), render at the
  well's own pixel size, opaque paint, caption top-right in WEAK_TEXT on a
  dimmed plate. The column is an `egui::Panel::right` *nested in the chat
  tab* (resizable, 220 default / 60 min / 70% max, 8px left margin so its
  resize grab zone clears the transcript's scrollbar). Viewer mesh-cache
  prune keeps activity-card scenes alive too. View ▸ Agent Activity toggles
  it (session-local; off = drain-and-drop, no column). Burst coalescing (N
  rays → one card) deliberately out of scope, the caps bound bursts.
  Commands: status/inspect/render/raycast/clearance, and poll/say/ack (see
  "Talking to the agent"); every command except those three syncs first
  (no standalone `sync` — folded into `status` 2026-08). Commands run
  concurrently (one thread per connection, no global command lock since
  2026-08-17): the one global lock is `EngineState::build_gate`, an RwLock
  expressing `Store::gc`'s quiescence requirement — passes (`query_view`,
  `build_slot`; covers meta extraction, which also evaluates JS against the
  store) hold it shared, publishing takes it exclusive around gc. So a CLI
  query never waits on another view's build (only its own, plus gc's few
  ms). Post-build store reads (inspect/flatten/render) are gate-free but
  hold a `Store::pin_root` guard (`query_view` takes it under the gate):
  a one-off build's root is otherwise pinned only by its memo entry, and
  memo entries can be LRU-evicted at any time (per-key and global caps),
  after which a publish's gc would sweep the scene mid-read. (Published slot roots are pinned as
  generation roots instead.) The renderer has its own mutex (renders
  serialize with each other only). One grammar
  (2026-08, plans/cli-json-args.md): a CLI command's argument is the JSON
  request body itself; `requests.rs` holds the serde structs *and* the
  field-spec table that validates names (siblings listed on a typo,
  removed commands redirected) and prints `docs/cli.md`'s generated
  per-command reference (drift-tested; regen `UPDATE_CLI_DOCS=1 cargo
  test -p odm-engine cli_reference`). `inspect` is the one scene query
  (see "Scene query" below); its root entry also carries the view
  interface on request (`"fields": ["description", "inputs",
  "presets"]` — what `build` used to answer), `"stats": true` adds
  build stats to any view command, and a *failed* build still attaches
  the target's declared schema next to the error (`CmdError::extra` →
  top-level fields). Input lints ride the `warnings` channel of every
  view-targeting success. View-scoped queries take optional
  `path` + `inputs`/`preset`, or adopt a viewer tab: `"view": true` =
  the user's active tab, `"view": "<slot>"` = that tab (untagged
  `ViewSel`); poll answers carry a snapshot of the user's active view
  (path, inputs, selection), and `status` reports per-slot inputs,
  build state (ok/error/building/pending, last-published — status never
  builds) and the active tab's selection. Agent-visible responses print
  no content hashes and no `generation` (status keeps it) — see
  notes/agent-surface.md. CLI one-off views build without publishing;
  viewer slots publish into a per-slot map (all live roots pinned together
  for GC). Protocol: ndjson over unix socket,
  `{ok: bool, ...}` responses. Files: `state.rs` (slot-keyed published map,
  active views + build queue, `build_slot`/`build_once`, and `stop`, below),
  `commands.rs` (the whole JSON
  layer: a serde-tagged `Request` enum with `deny_unknown_fields`, so a
  typo'd command *or* option is an error, plus `CmdError`→JSON),
  `watcher.rs`, `server.rs`, `session.rs`, `scene.rs`, and `viewer/` — the
  desktop chrome around odm-viewer-core (`mod.rs` shell: layout, tab strip,
  chat, dialogs, and the `impl odm_viewer_core::Engine for EngineState`;
  `idle.rs` event loop, `menu.rs` menu bar + its accelerators, `browse.rs`
  folder list with `open.rs`/`new.rs` on top of it, `pick.rs` doohickey
  picker, `tabs.rs` tab persistence, `agent.rs` agent-file question, `activity.rs` — see "Agent activity view" above).
  odm-viewer-core's `theme/` holds the viewer's dark Windows 95
  look (classic bevel structure, inverted luminance, white text):
  a `Style`/`Visuals` preset plus widget wrappers (`button`,
  `collapsing`, `list_box`/`sheet_box`, `text_edit`, `trackbar`, `check_box`,
  `radio`, `reset_button`, `menu_bar`/`menu`,
  `dialog`, `list_row`, `tab`/`tab_edge`, …) that paint two-tone 3D bevels —
  egui's `WidgetVisuals` has one uniform `bg_stroke`, so bevels can't be
  themed and must be drawn over each widget's rect. Prefer these wrappers over
  bare `ui.button`/`egui::ScrollArea`/`egui::CollapsingHeader`/
  `egui::Slider`/`ui.menu_button` in viewer code — egui's own are all off-theme
  (rounded, hover lit, anti-aliased glyphs, twisty arrows). Two standing rules:
  no animation (`animation_time = 0`, `ScrollAnimation::none()`, no scroll-edge
  fade, no busy spinner — state changes snap), and no hover feedback — the
  exceptions being drop-down items, which highlight because that is how a menu
  is read while dragging through it, and a tab's close box, which is too small
  a target not to say when it is armed.
  Text is bundled bitmap fonts and tree icons are bundled pixel art, not system
  ones — see "Viewer fonts" and "Viewer icons" below; small glyphs that are not
  worth a file (the checkmark, scrollbar arrows, the tree's +/-) are painted
  from `theme::pixels`/`theme::arrow` as a `Mesh`. See "Scrollbars" below.
- `odm-export` + `odm-web` + `xtask` — `odm export --web`: the native
  bundler/manifest half, the wasm browser host, and the template builder
  (`cargo xtask build-web-template`). Whole story in
  notes/web-export.md; odm-web is wasm-only (native = empty stub) so
  ordinary builds never need its toolchain.
- `odm-prompt` — std-only, no odm deps (odm-cli must stay V8-free): the
  `docs/prompts/*.md` `include_str!`s behind `text()`, plus the marked-block
  machinery both the `prompt` docs topic and the engine's on-open sync use.
- `odm-cli` — transport only: project resolution — `find_project` (the
  cwd walk-up), `project_dir`, `is_project`, all reused by `run` — the
  socket, one JSON positional forwarded as the request body (plus
  poll's two flags and say's free text; the engine owns all other
  validation), client-side `out` resolution, pretty-printing, the poll
  ack handshake, exit code from `ok`. Plus the engine-less `docs.rs`
  (`odm docs` `include_dir!`s the whole `docs/` tree: topic dump —
  including the `prompt` topic over `odm_prompt::text` — section-grepping
  `search`, `changes <from> <to>` migration concatenation, `--api N`
  rejected until frozen docs snapshots exist).
- `odm` — the only binary. `odm run [<dir>] [--headless]` → the engine
  (`odm_cli::project_dir`, per Project format above: the dir named or cwd,
  never an ancestor); `odm export --web <out>` → odm-export (standalone,
  no engine); everything else → `odm_cli::run`. Top-level `--help` splices in
  `odm_cli::USAGE`. Splitting the two halves into libs behind one bin keeps
  the client's dependency-light layering and leaves room for a
  client-only build later, while shipping one binary: no CLI/engine version
  skew, one `--help`, and a place to hang engine auto-start if we want it.
  Costs measured before merging: +1.7ms per client invocation (0.66→2.4ms,
  the binary is ~500MB in debug), and a touched-CLI relink goes 0.22s→1.05s.

### Tab strip

`ViewerApp::tab_bar` lays the whole strip out by hand (`theme::tab` paints one
tab, `theme::tab_edge` the page edge under them). Three things it depends on:
the paint order — unselected tabs, then the edge cutting them off, then the
selected tab (grown 2px on three sides) cutting the edge —, the close box being
carved *out* of the tab's click rect rather than layered over it, so neither
steals the other's click, and the + being pinned to the right end when the row
overruns, so a full strip can still be added to. Labels elide
(`TextWrapping::truncate_at_width`) once tabs are squeezed past their share.

### Input panel

odm-viewer-core `inputs.rs` + `tab.rs`. Shape (reworked 2026-08-17,
twice): every input is a **block**, not a row — its name on a row of its
own, its control(s) beneath at full panel width. The exception is a
boolean, whose box goes first with the name beside it (a box wants its
label next to it, and the whole row is the click target). The reset
button ends the *name* row (a hooked revert arrow — a circle arrow has
too few pixels to read as anything but a dot), drawn only for a pinned
value. Per type:

- boolean → `theme::check_box`; enum → `theme::radio` rows, each row
  spanning the panel so the empty space right of a choice selects it.
- number/integer → a text field plus, at its right, a `theme::trackbar`
  when both `minimum` and `maximum` are declared, else the four
  adjusters `/2 - + 2x` (`-`/`+` step by the ten's place below the
  value's own, so one click nudges at any magnitude; integers step by 1;
  a declared min/max clamps).
- vector2/vector3/quaternion → one number row per component under an
  x/y/z/w letter; matrix4 → a 4x4 grid of bare fields (sixteen adjusters
  is too much), laid out **as the matrix reads** (row r, column c =
  element `c * 4 + r`) over the column-major wire form.
- **structure recurses** (2026-08-17, was plans/structured-inputs.md):
  `object`+`properties` → one labelled sub-block per property (the
  top-level block shape, indented `INDENT`=10px per level; bools keep
  the box-beside-label row); `array`+`items` → per-element rows
  (single-row leaves inline beside the index label, structured elements
  as an indented block) each with a `theme::remove_button` ×, plus Add
  (new element = `odm_build::synthesize(items)` — items.default, else
  nested defaults/type-blanks/required fills); `variants` unions → tag
  as radio rows, the active variant's properties beneath, switching
  replaces the subtree with `synthesize_variant` (shared fields belong
  outside the union); `object`+`additionalProperties` maps → the array
  control with an editable key column (key-sorted display; rename =
  remove+insert, empty/duplicate discards; Add picks `new`/`new-2`/…
  and focuses the key field). Anything unrenderable (untyped, `object`
  with neither properties nor additionalProperties, array without
  items) → the JSON text field for that subtree.
- Numbers show rounded to 4 decimals.

Addressing: a leaf is `inputs::Field { section, name, path, slot }` —
`path` = `Vec<Seg>` (`Key`/`Index`) from the input root, `slot` =
`Component(i)` (vectors/matrices spread one leaf over fields) or
`MapKey`. Widget ids carry the path. **Events stay whole-value**:
`Event::Set(section, name, value)` splices the leaf edit into a clone
of the shown top value (`splice` creates intermediate containers), so
pin/reset/`set_or_clear`/persistence are untouched and comparison stays
whole-value — growing an array pins it, shrinking back to the default
clears. Shown leaf value = navigate the top value by path, else
`synthesize(node schema)` (nested default, else type-blank). Reset
stays one button per top-level input.

A text field **commits on losing focus**, not just on Enter — clicking
away applies what you typed; only Escape discards (egui's own
`DragValue` uses the same `lost_focus() && !escape` rule). Number fields
that don't parse as a number discard instead. `num()` writes integral
values as JSON integers, so a field or slider never turns `42` into
`42.0` in the view's args.

`Reset` is the first button of the preset row: same look, and its bundle
is "every default" — `Event::ClearAll` empties `set_args` and
`set_cascade`, which is exactly resetting each field individually.
Preset buttons flow onto rows by hand — egui's `horizontal_wrapped`
either letter-wraps a button's text or (with `TextWrapMode::Extend`)
overflows and *widens the scroll content*, poisoning `available_width`
for every later row.

`t` is an input like any other, in the panel with the rest (its bottom
"timeline" panel is gone, 2026-08-17, on the user's call: it is just a
ranged cascade number). The one special case is the Play/Stop button
parked at the right end of its row; `Viewer::advance_transport` still
drives it at 1 unit/sec, looping.

Wanted later: a color swatch/picker; element reorder buttons and
per-path reset; selection↔panel linking (authored provenance tags
mapping scene nodes → input paths, needs an IR field + FORMAT_VERSION
bump — wait for demonstrated need); doohickey-typed inputs (`items`
referencing another file's declared inputs would de-duplicate the
gallery's marker schema and make a real scene composer).

`examples/input-gallery` is the panel's fixture project: one input per
control it can draw (every extension type, an enum on a non-string type,
array/object/map/union/untyped inputs, presets, the `t` transport, a
cascade input declared only in `parts/`, and the headline `objects`
array — one `parts/marker.js` invoke per element, so editing one
element memo-hits the rest; `input_gallery_object_edits_memo_hit_the_rest`
guards the pattern). Open it when changing this file;
`examples::input_gallery_covers_every_control` guards the report side.
Known gap it makes obvious: long values still clip in matrix cells and
in the array/object text fields (issues/panel-clips-long-input-values).

Controls come from the tab's
fall-through report; interactions come back as `Event`s which
`inputs::apply` folds into the tab's `set_args`/`set_cascade` (pure and
unit-tested; `Tab::apply` then submits `tab.view()` through the `Engine`
trait). The
invariant that keeps it honest: **the panel is a pure render of (report, tab
set values)** — the only other state is `Tab::edit`, the buffer of the one
text field currently holding keyboard focus (egui focus is single, so it's
an `Option<(Field, String)>` — `Field` being input name + component index,
present exactly while focused; Enter and any other focus loss apply,
Escape discards). `panel_ui` *takes* the edit out of the tab for the frame
and puts back what still has focus, so a field that loses focus finds its
buffer whatever order the fields draw in. Two rules that came out of a 2026-08-14 bug (a text field
frozen at its first-frame value, blind to presets/resets):

- Never cache what a control displays outside `Tab::edit`. Unfocused text
  fields re-derive their string from `shown_value` every frame.
- `shown_value` = the tab's set value, else the entry's declared *default* —
  never the report's resolved `value`, which is from the last successful
  build and lags the set values (briefly after any change, forever if the
  build fails). The tab is the only writer of view-level values, so unset
  always means "resolves to the default".

Two behaviors added 2026-08-17 (issues/stale-pinned-view-inputs):

- Setting a value equal to its declared default *clears* the pin instead
  (`inputs::set_or_clear`, numbers compared numerically), and the reset
  button only enables for a real difference — "set to the default" and
  "cleared" are one state, in the tab, the view identity, and viewer.json
  alike.
- Stale pinned args (the target dropped/renamed an input) self-heal: a
  failed slot build publishes the target's declared inputs
  (`Published.declared`, cleared on success), `Tab::prune_stale_args` drops
  pinned args no longer declared plain — only when the failure was built
  from the tab's current values, and never touching cascade values — and
  `poll_published` resubmits (returns true = host persists tabs). The
  build error itself reports *all* unknown view args in one message,
  blamed on the view's pinned values, with cascade inputs listed as
  settable (`effective_args`'s `at_view`).

Text fields get stable explicit egui ids (`("input", section, name)` via
`theme::text_edit`'s id param) so focus/cursor state survives rows
appearing above them — and so tests can find them. `inputs.rs` tests drive
the real panel through a headless `egui::Context` (real clicks on painted
labels, real focus, asserting on the painted galley text).

### Menu bar, and switching projects

`viewer/menu.rs` is the whole bar: an `Action` enum, a `theme::menu` per
drop-down listing `MenuEntry`s, and one `apply` that turns an action into an
effect. File has New Project… / Open Project… | Open Doohickey… (Ctrl+O) /
Close Doohickey (Ctrl+W) | Export Web… | Quit (Ctrl+Q); Edit has one item,
Message Agent (Ctrl+Enter) — put the dock on the Agent tab and the caret in
its box (`ViewerApp::focus_chat`, taken by the box when it next draws); View
has Frame Scene (F) and checkmarked Wireframe/X-Ray/Grid/Agent Activity.
Opening a *project* has no chord on purpose: swapping the whole engine is not
something to trip over next to Ctrl+O. `menu::shortcuts` reads those chords
off the input queue at the top of the frame and `apply`s the same actions —
consumed, so a chord never also lands in whatever has the caret (Ctrl+Enter
has to beat the chat box it aims at). A modal
owns the keyboard while it is up (only Quit still fires). `theme::menu` measures its own entries and pins the popup width
before drawing, because an auto-sizing egui popup doesn't know its width until
the frame after — and a highlight that stops at the text looks broken. Titles
are painted by hand (open = filled with `ACCENT`, never pressed in) rather than
via egui's `MenuButton`, and `theme::menu_bar` does the hand-over itself: each
title records its rect, and while a menu is up the bar opens whichever title
the pointer is over — decided before any title draws, so two menus are never
painted at once.

File ▸ Open opens **another engine**, it does not reconfigure this one: an
engine is bound to one project's store, socket, build loop and watcher.
`session.rs` holds the current `EngineState` (there may be none — see below)
and swaps it:

- The V8 snapshot (`Arc<JsEnv>`) is the one thing shared across sessions —
  `JsEnv::new` is a once-per-process job, and building a second one on the UI
  thread while a build thread holds isolates is asking for trouble.
- The new project's socket is claimed *first*. Binding is what fails when the
  project is already served, so a failed open leaves the running one untouched
  and the dialog up saying why.
- The old session is then `stop()`ed: a flag plus wake-up hooks registered by
  whoever can block (`server.rs` connects to its own socket to break `accept`,
  `watcher.rs` sends on its channel, the build loop gets a condvar notify).
  Each thread returns and drops its `EngineState` share; the retired socket
  file is removed so the CLI fails fast instead of hanging on a dead engine.
  Verified: two swaps leave the same 45 threads and idle CPU as a fresh start.
- The viewer then blanks itself — scene, tree, selection, timeline, GPU mesh
  cache — and reframes on the first build, exactly as at startup.

**No project open** is a real state, not just a moment during startup:
`Sessions::empty()` builds the snapshot and serves nothing, and the viewer
launched outside a project starts there (`ViewerApp::session: Option`). It
draws the menu bar (File only — nothing to look at, so no View menu) over a
"No project open." panel with Open Project… / New Project… buttons, with the
Open dialog already up, browsing cwd. Cancel leaves the panel rather than trapping the
user in a modal with nowhere to go. `ViewerApp::state()` expects a session,
which holds because `ui` peels this case off first; everything downstream of
it (tabs, `self.tabs[self.active]`) assumes a project. Open from here is the
same `Sessions::open` path as a swap, minus the old session to retire —
`Sessions::start` is now just `empty()` + `open()`.

`viewer/pick.rs` is the doohickey picker — the tab strip's magnifier and File ▸
Open Doohickey both put it up, on a fresh `sync()` so it lists what is on disk
now. An auto-focused filter box over the list: case-insensitive substring,
up/down walk the highlight, Enter takes it, a click takes the row clicked. The
keys are consumed before the box is drawn, or Enter would drop its focus and
the arrows would move its caret.

`viewer/browse.rs` is the directory chooser both project dialogs are built on
(no portal here, no dialog crate in the tree): "Look in:" + Up over a
`theme::list_box` of subdirectories — directories only, since a project *is*
one, hidden ones skipped, and `is_project` (odm.toml) picking the icon. It
owns the dialogs' one message line (`report`), and hands back what a row was
asked to do (`Hit`) rather than acting, since navigating mid-layout would
relist under the rows still being iterated. `beside(current)` browses
`current`'s parent with it selected; `at(dir)` browses `dir` with nothing
selected, for when there is no project to start from.

`open.rs` adds the path field, which is what Open acts on, so clicking a row
and typing a path are the same gesture; double-clicking a plain folder browses
into it, double-clicking a project opens it. `~` expands, nothing else does.

`new.rs` adds a *name* field instead — the project is a folder that isn't
there yet, so there is nothing to point at. It refuses a name with a `/` in
it, a leading `.`, one that is already taken, and a folder inside a project
(every .js under a project belongs to it, so nesting would build the inner one
as part of its host). Create writes the project (`odm_build::create_project`:
odm.toml + a starter `root.js`, never over an existing file) and opens it in
one gesture. Written-but-not-opened — the socket claim can fail — hands off to
the Open dialog pointed at the new project, since making it is done and only
opening is left.

The two are one `Option<Dialog>` on `ViewerApp`: only one is ever up, both are
drawn last (their backdrop covers everything above), and both keep themselves
up with the reason when the engine turns a pick down.

### Talking to the agent

The user types in the viewer's chat panel; the agent collects messages with
`odm poll` and answers with `odm say`. No MCP: CLI + `docs/prompts/` is
agent-agnostic and enough.

- **The message box takes newlines** (2026-08-17, was
  issues/chat-box-no-multiline.md): `theme::text_area` — a multiline
  `TextEdit` whose `return_key` is *shift+Enter*, so plain Enter falls
  through for the caller to act on (`TextArea::submitted`). ctrl+J, what
  terminals bind, is rewritten into a shift+Enter event before the widget
  runs, so egui's own handler does the insert (caret, selection, undo). Two
  traps: the submit check must read the *event's* modifiers, not
  `InputState::modifiers` (a rewritten ctrl+J is an Enter press with ctrl
  physically down), and egui multiline no longer surrenders focus on Enter,
  so `lost_focus()` is not the signal it was. The box grows with its text —
  `theme::text_area_height` lays the galley out ahead of the widget, since
  the transcript above has to be sized before it is drawn — capped at 8 rows
  or half the chat, whichever is less; past that it scrolls (egui keeps the
  caret in view on the next keystroke). Transcript indents a message's later
  lines under the `>`.

- **The working status** (2026-08-17): `odm say --task <text>` sets the one
  live "working on" line; `--done [<text>]` clears it, posting any text as a
  normal message. A user message sets it to `state::PROCESSING`
  ("Processing") on the spot, so the viewer reacts on Enter instead of
  waiting for the agent's first `--task`; the agent's own task replaces it
  and `--done` clears it. The prompt tells the agent to always clear it
  before it stops. Stored as a last-write-wins `Option<String>` in `Chat`
  (state.rs `set_task`/`clear_task`/`task`) — a status value, no Delivery
  machinery, works headless. The viewer draws it as a dim-blue tail line in
  the chat transcript with era busy-dots cycling at 0.4s (repaint timer only
  while set — the one exception to theme's "no animation anywhere").
  Deliberately **no expiry**: a timeout would fake "done" during long
  thinking. Instead every say/poll/status response echoes a standing `task`,
  so the agent (or a successor) sees a stale one in-band and clears it; the
  prompt tells it to. Exclusivity (`task` vs `text`/`done`) and empty-text
  are `cmd_say`'s to enforce, not serde's.

- **The agent's actions are log lines** (2026-08-17, `Who::Action`,
  `EngineState::log_action`): one compact transcript line per command it ran
  (`dispatch` logs the verb + the response's resolved `view` path, or
  `verb failed: <first line, clipped>`; poll/say/ack are the chat, not
  actions) and per file that changed (`note_generation` diffs the last sync's
  `generation_sources` against the new one — `new`/`edit`/`deleted`, or one
  "N files changed" line past `FILE_LOG_CAP`, and never for a session's first
  sync). `Delivery::Done` from birth, so no poll ever takes one, and gated on
  `viewer_attached()` like activity events — headless keeps no log. `status`
  now calls `note_generation` too (it syncs without building; a generation it
  was first to scan must not be swallowed — slots go stale, edits get logged).

- **Poll's contract is set by agent harnesses.** The baseline one can't read a
  running background command's output — it is woken when the command *exits*.
  So poll blocks until ≥1 message is queued, prints them all, and exits;
  process exit is the delivery mechanism. `--timeout` bounds the wait (empty
  `messages`), and a retired session (File ▸ Open) answers `{"ok": false,
  "error": {"kind": "stopped"}}` so a poll never outlives its engine.
- **`--follow` is the same thing for harnesses that watch lines** (Claude
  Code's Monitor, say): park one command at session start and every batch
  arrives as a push — no relaunch per message, and no gap where nobody is
  listening. The loop stays CLI-side (`follow_poll`): send poll, print the
  response as one compact JSON line, flush, ack, poll again — so the
  protocol's one-response-per-request rule is untouched, and delivery is
  the same two-phase handshake per batch. `--follow` additionally sets the
  request's `events` flag (2026-08-17, the one revision of "`--follow`
  never reaches the engine"): the engine must know to answer on diagnostic
  value changes too, not just messages. A `--timeout` alongside it is an
  error (nothing to bound). `docs/prompts/cli.md` tells the agent to park a
  follow if its harness can watch lines, and to loop `--timeout` otherwise.
- **Build diagnostics ride poll** (2026-08-17, plans/js-diagnostics.md):
  every poll response carries `builds` (per active slot: `build` =
  ok/error/pending + `error` + `stale`) and `health` (failures-only list
  from the background sweep), read from `published`/`health` at answer
  time — never the build gate. With `events`, a blocked poll also returns
  (possibly empty `messages`) whenever the *diagnostic value* — the map of
  failing slots/files → error, `EngineState::diagnostic_map` — differs
  from what the connection last reported (`Conn::events_baseline`).
  State-compare on every wake (publishes, sweep stores, `remove_view` and
  new generations call `wake_pollers`), not an event queue: missed/spurious
  wakes and reconnects can neither lose nor duplicate; ok→ok rebuilds and
  stale flips don't emit. Logs never ride poll — query the view for
  error + logs (memoized replay is byte-identical to a re-run).
- **The health sweep** (state.rs `SweepState`/`sweep_one`): per generation
  the build loop, when no slot is queued, meta-checks every file and
  builds the default view of every standalone-buildable one (all
  non-cascade inputs have defaults; others get the meta tier only). Slot
  builds preempt an in-flight sweep pass (cancelled item requeues; a new
  generation supersedes the queue wholesale). Results land in a per-file
  `health` map (value + generation; older generation ⇔ reported with
  `stale: true`). Failures-only in every surface; a file's absence claims
  nothing, and per the per-view axiom an ok default view never precludes a
  slot failing at other inputs. Broken files replay from the failure memo,
  so repeated sweeps of them are cheap.
- **Engine host warnings are transcript entries** (`Who::Engine`): watcher
  creation/watch failures, open-time odm.toml/prompt-sync warnings (queued
  right after `EngineState` construction — `session::OpenScan`), server
  death. Same Pending→InFlight→Done handshake as user messages; on the
  wire they're `"from": "engine"` (absence = user); the viewer renders
  them `engine: …` in `theme::WARN`. stderr prints stay for daemon logs.
  Session events, not build output — deliberately not in the console pane.
- **Delivery is committed, not assumed** (the fix for a 2026-07-27 bug where
  Ctrl+C on a poll made the next message disappear). Each entry carries a
  `Delivery`: `Pending` → `InFlight` (a poll took it) → `Done`, and *only* an
  `ack` from the client moves it to `Done`. The CLI sends that ack after it has
  printed and flushed the messages, so the engine's copy is never the only one
  in flight. Every other ending — Ctrl+C, broken pipe, crashed harness, a
  connection that just closes — drops the `Conn`, whose `Drop` returns anything
  unacked to `Pending`. Failure therefore duplicates rather than loses, which
  is the direction to fail in; `docs/prompts/cli.md` warns the agent about repeats.
  The viewer dims anything not `Done`, so an undelivered message still looks
  like one.
- **A blocked poll must notice its client dying.** Reading is on its own
  thread per connection (`read_requests`), so EOF is seen while the handler
  blocks; it sets `Peer::gone` and wakes the pollers, which return
  `Disconnected`. Without this the connection thread parks on the condvar
  forever: `listeners` stays wrong ("agent is listening" with nobody there) and
  the zombie wins the next batch. `server::tests` reproduces exactly that over
  a real socket.
- `state.rs` holds the queue: a `Chat` of one transcript `Vec` under a mutex —
  the pending entries *are* the queue, so there is no second list to fall out
  of step with it — plus a condvar and a `listeners` count (blocked polls). In
  memory only; the agent's own conversation is the durable record.
- **`poll`/`say` never sync or build** (and never touch the build gate): a
  poll blocks for minutes, and must hold up nothing.
  `state::tests::chat_commands_skip_the_build_gate` guards it.
- Viewer: the dock's Agent tab —
  `theme::tail_box` transcript (user lines `> …` in `theme::USER_TEXT` blue,
  dimmed while undelivered, but white as they are typed in the input box;
  agent lines white; action lines gray in `theme::ACTION_TEXT`; the live task
  line green in `theme::TASK_TEXT`) plus one `theme::text_edit`
  where Enter sends and keeps focus. The transcript takes the panel's height
  less the input line, exactly (item spacing included) — get that arithmetic
  wrong and the panel grows a few px every frame until it eats the window.
  The lamp on the tab says whether the agent is listening, which is the user's
  cue to go prod it in its own terminal. All of it repaints through the
  existing `EngineState::wake`.

### Owning the event loop

`viewer/idle.rs` wraps eframe's winit application (`SlowIdle`) to fix two things
eframe leaves broken here:

- **Idle spin.** When a repaint falls due eframe parks the loop in
  `ControlFlow::Poll` expecting `RedrawRequested` straight back; an undisplayed
  Wayland surface never gets its frame callback, so `Poll` pegs a core. Whenever
  eframe leaves `Poll` (or its own expired `WaitUntil`) set, we clamp it to a
  100 ms timer. Events still wake the loop instantly, so a visible window is
  unaffected.
- **Never exiting.** On a close request eframe destroys its windows and then
  waits for *another* window event before deciding to exit — one a destroyed
  Wayland surface will never send, so the process sat in `epoll` forever with
  nothing on screen. `SlowIdle` watches for `CloseRequested` itself, and
  File ▸ Quit (Ctrl+Q) sets the same shared `Quit` flag; `about_to_wait` acts on it.
  Both paths verified to reach `process::exit(0)`.

### Viewer fonts

`crates/odm-viewer-core/assets/fonts/` holds two bitmap faces converted from
X11
fonts by `scripts/bdf2ttf.py` — `odm-sans-14` (Adobe helvR10, the
period-correct MS Sans Serif lineage) everywhere, `odm-mono-14` (misc-fixed
7x14) in the build-error panel. Sources, licenses, available strikes,
coverage, and regeneration live in the README next to them. `theme.rs`
`include_bytes!`s both at the front of egui's Proportional/Monospace lists
(built-ins stay as fallback). Pixel-grid consequences: `theme::UI_SIZE`/
`CODE_SIZE` are pinned to 14 and every text style uses them — resizing the UI
means regenerating from a different BDF strike, not typing a new number; both
faces set `FontTweak { hinting: false, subpixel_binning: false }`;
whole-number `pixels_per_point` scales fine, fractional blurs.

### Viewer icons

`crates/odm-viewer-core/assets/icons/` — one 11×11 RGBA PNG per icon,
`include_bytes!`d by `icons.rs`, uploaded once, drawn as one NEAREST-sampled
quad: left of each tree name via `theme::tree_row` (`empty`/`mesh`), and left
of each Open-dialog row via `theme::list_row` (`folder`/`project`). Editing
workflow
(`scripts/icon-png.py` converts PNG ↔ text grid), color constraints, and
adding an icon are in the README next to the art. Rules that bite in viewer
code: whole pixels only (`icons::SCALE` is an integer; positions go through
`theme::snap`), and never paint pixel art as individual rects — egui replaces
rects thinner than 2px with feathered line segments. Where a texture is
overkill (the tree's dotted lines), hand the pixels to `Painter::add` as a
`Mesh` of `add_colored_rect`s, which skips tessellation and so skips
feathering.

### Scene tree

`theme::tree_row` draws a whole row — nesting gutter, icon, name — and
odm-viewer-core `tree.rs` walks the tree telling it where each row sits (depth, which
ancestors still have siblings below, whether this row is the last of its own).
It walks a viewer-local `TreeNode` snapshot (name/has_mesh/children),
materialized from the store once per published build, since IR children are
hashes; a store miss takes the same retry-repaint path as a failed flatten.
The gutter is the era's registry-tree look: 1px dotted lines on a
`(x + y) even` checkerboard of the screen, and a boxed `+`/`-` where a node has
children. Consequences:

- Rows must abut, so `tree_ui` zeroes `item_spacing.y` and the padding lives in
  the row instead — a gap would break the dotted lines between rows.
- `TREE_INDENT` is even and the row midline is nudged onto the checkerboard, so
  every column and rule shares a parity and corners get a dot.
- The +/- hit target is ours, and so is open/closed state: `TreeState` in
  odm-viewer-core tree.rs, not egui's `CollapsingState`. The box toggles, the name selects,
  a double-click on the name does both.
- Selecting a node auto-expands its ancestors, and collapsing them again when
  the selection goes away is why the state is ours: `TreeState::auto` remembers
  what each auto-expand displaced, and any user toggle (`set_manual`) takes
  that node out of auto-expand's hands for good. Nodes above `AUTO_DEPTH`
  start open.
- F (View ▸ Frame) fits the selection's world AABB if anything is selected,
  else the whole scene's (`odm_render::subset_bounds`), keeping the current
  yaw/pitch. It moves the orbit *target*, so the camera keeps turning around
  what was framed after the selection is dropped, until F with an empty
  selection recenters on everything.
- Selection is a list, in pick order. Shift-clicking a row — or a solid in the
  viewport — adds it, or removes it if it was already selected; a plain click
  replaces the whole selection. `status` (active slot) and poll
  snapshots report the list.
- odm-viewer-core's `tree::tests` drives rows through a headless `egui::Context` (real hit
  testing, real modifiers — note egui reads `modifiers` off `RawInput`, not
  off the events). That is how modifier-clicks are *tested*; injecting one into
  a live viewer also works, but only as a chained call (see "Seeing the
  viewer").

### Scrollbars

`theme/scroll.rs`. egui's scrollbar can't be themed (one uniform `bg_stroke`,
so no bevel) and has no arrow buttons, so `theme::list_box` — the sunken well
every scrolling pane lives in — hides egui's bar and paints the era's:
square arrow buttons, a 50% dithered trough, a raised handle. egui still owns
the scrolling; we read `ScrollAreaOutput` and write `State::offset` back.
Consequences:

- `sheet_box` is the same well filled with the control face instead of the
  window colour, for panes whose contents are themselves controls (the inputs
  panel): the sunken dark fills belong to the fields and radios inside it.
- The bar is always present on an axis it was given, graying its arrows when
  there is nothing to scroll. That means the pane's size is fixed by the
  caller (`ERROR_HEIGHT` for the error pane, `LIST_HEIGHT` for the Open
  dialog's list), not by its contents.
- The trough is a 2×2 texture with `TextureWrapMode::Repeat`, uv'd from screen
  pixels so it lands on the same checkerboard as the tree's dots. A mesh of
  single pixels would be thousands of rects for one tall bar.
- The handle is registered *after* the trough so that egui's hit test gives a
  press on both to the handle. Dragging maps pointer→offset absolutely
  (`handle_span`/`offset_at`), so the handle stays under the grabbed point
  after the offset clamps at an end.
- Arrows and trough auto-repeat while held (`pressed`), and fire once on a
  click whose press and release land in the same frame — with repaint-on-demand
  a quick click really is one frame.
- `theme::scroll::tests` drives all of this through a headless
  `egui::Context`, like the tree tests. The GUI harness *cannot*: wdotool
  delivers a whole press-move-release chain inside one frame, so an injected
  drag never registers. Screenshot the look there, test behavior in unit tests.

### Scene query

`odm inspect` is the whole scene-query surface (it replaced a separate
`tree` in 2026-08 — one query at two hardcoded corners of a scope × detail
grid, with mismatched field names). Two orthogonal knobs, both defaulting
off one signal — *did you name a node?*:

- **scope**: `node` (name, or index path as tiebreaker) plus
  `depth`/`recursive`. Unnamed → whole scene, recursive; named →
  that node, children as a count.
- **detail**: summary (unnamed) | full (named, or `"full": true`) |
  `"fields": [...]`. `id` and `children` are structural and always
  there. View-level fields (`description`/`inputs`/`presets`) land on
  the root entry only; a fields list with only view-level names also
  collapses depth to 0.

Decisions worth keeping (`scene.rs`):

- **`bounds`/`tris`/`verts` are subtree aggregates**, computed without the
  kernel (stored mesh AABBs + counts, cached per mesh hash). That is what
  makes "how tall is the whole model" answerable at any elision point, and
  what keeps a recursive summary cheap. `volume`/`area` are per-mesh and do
  hit Manifold — full detail only, on demand.
- **`volume`/`area` are world-space**: scaled by s³/s² under a similarity,
  and measured on the transformed solid under shear/non-uniform scale
  rather than quietly reporting local numbers.
- **Runs of identical consecutive siblings collapse to `repeat: N`** —
  identity is the node's content hash with its transform zeroed, so whole
  repeated subtrees collapse, not just meshes. The entry shown is the run's
  first member; ids run on consecutively. Collapsing switches off as soon
  as the requested fields would show placement (`position`/`matrix`/
  `world_matrix`), so `"full": true` expands.
- **One node schema everywhere**: `id` + `name` in `inspect`, `raycast`
  hits (whose hit position is `point`, as in JS) and selection lists
  alike.
- Dropped in the merge: mesh hashes (internal; `repeat` delivers their one
  payoff) and `bounds_local` (a JS-side concern).

The CLI prints responses with its own `pretty` (odm-cli `lib.rs`): standard
indentation, except that any value fitting in 96 columns stays on one line.
A 3-vector as five lines was a large share of the 191 KB tree that started
this.

## API versions

Contract: `docs/versioning.md`; rationale + implementation map:
`notes/api-stability-and-docs.md`. The short version: every doohickey
carries `//! odm <version>` (parsed at sync time in odm-build/sources.rs,
missing = unstable until v1); `framework/versions/<v>` manifests register
per-version installers in one shared snapshot, `run_build` installs the
selected surface into each isolate before its module loads, and bare
`'odm'`/`'three'` imports resolve per version. A test-only `test` version
(feature `test-api-version`, dev-deps only) keeps the machinery honest.
One snapshot per process is a hard V8 constraint, not a choice — see
notes/spike-findings.md "Snapshot count/concurrency".

## Invariants & policies

- Consistency: every published result is byte-equivalent to a from-scratch
  build of its generation (tested: `odm-build/tests/build.rs`).
- The engine never writes ODM project files of an existing project — with two
  exceptions, neither of which can touch a doohickey or churn a generation
  (neither odm.toml nor `.md` files are part of generation identity):
  the `engine` value in `odm.toml` (recorded on project open;
  `odm_build::sync_marker`), and the standard prompt inside the marker pair of
  an agent file (`odm_prompt::sync` — both run from `session::sync_on_open`;
  see "Agent files" under Project format). Agent-file writes are either marker-scoped (the markers *are* the
  file's opt-in) or user-consented (a viewer question box). (`create_project`,
  File ▸ New Project, authors a project's first files, but only ever creates
  files that are not there — it may target an existing folder, where the New
  dialog's empty Name field means "this folder", named after its leaf; only
  `odm.toml` already existing is an error, an existing root.js/agent file is
  kept.)
- Engine queries on content-addressed handles are pure → never memo deps.
  Queries on transformed solids bake via op_transform_bake (cached per
  Solid) — exact, but costs a mesh copy per distinct transform.
- IR-hash goldens (`odm-build/tests/examples.rs`): regenerate on V8/three/
  Manifold upgrades (run the test, copy printed values).
- Golden PNGs: only meaningful per-adapter; plan was lavapipe+pinned-Mesa in
  CI — CI deferred by user, so none exist yet.
- Version pins that move together: egui/eframe + wgpu (egui pins a wgpu
  major); deno_core + deno_error + v8. manifold-csg pinned =0.3.3.

## Testing

`cargo test` runs everything in ~1s after compile. Almost all tests are
integration tests in `crates/*/tests/`; the unit tests in `src/` are
`odm-render/src/grid.rs`, `odm-js/src/version.rs` (pragma parsing),
`odm-prompt/src/` (marker splicing + the agent-file scan); in
odm-viewer-core, `icons.rs`, `tree.rs`, `inputs.rs` (the input panel,
headless egui) and `theme/scroll.rs`; and in odm-engine, `commands.rs`,
`state.rs` (the chat queue), `server.rs` (delivery over a real socket) and
`conformance.rs` (the suite runner, below). Every
test binary shares one `JsEnv` in a `OnceLock` (`state::tests::env()` in
odm-engine) — building a snapshot while another test thread runs JS aborts
the process (see spike-findings "Snapshot count/concurrency").

Two data-driven suites guard the JS API:
- **Conformance**: `tests/conformance/unstable/` (repo root), run by
  `cargo test -p odm-engine conformance`. Declarative `export const
  checks` per test doohickey; format in `tests/conformance/README.md`.
  Add a test with every feature and every bug found — it seeds the frozen
  v1 suite.
- **Doctests**: every fenced ```js block under `docs/` must build
  (`cargo test -p odm-build --test suite doctests::`; ` ```js skip` opts out).
  Keep docs examples self-contained — free variables fail the build.

odm-build's integration tests are modules of one `tests/suite/` binary, not
separate `tests/*.rs` files — every test binary there links the full V8/engine
stack, so each extra file costs its own huge link. Filter with
`cargo test -p odm-build --test suite <mod>::`. Apply the same pattern if
another heavy-linking crate grows past a couple of test files.

Manifests suppress empty harness output: `doctest = false` on every lib (we
write no *Rust* doctests, and `odm-js` otherwise inherits an ignored one
from a deno_core macro), `test = false` on the `odm` bin and on the libs
with no `#[cfg(test)]` modules. **If you add unit tests to `src/` in
odm-build/odm-cli/odm-ir/odm-kernel/odm-store, flip that crate's `[lib]
test` back to true** — the manifest carries a comment saying so.

Useful invocations: `cargo test -p odm-build`, `cargo test --test render`,
`cargo test <substring>`, `cargo test -q` (dots instead of one line per test).

### Seeing the viewer

`odm render` only exercises `odm-render`, so viewer/theme changes need a real
screenshot. That is the **gui-testing** skill's job (`guibox` + `grim` +
`wdotool` in a private headless sway; AGENTS.md has the short version, the
skill's SKILL.md the rest). The repo only references it, from
`.claude/settings.json`; nothing ODM-side wraps it.

Do *not* retry the ambient display: the host's sway runs as root and we reach it
through a `wayland-root` socket symlink as uid 1006, where it advertises neither
`zwlr_screencopy_manager_v1` nor `ext_image_copy_capture_manager_v1`, so `grim`
fails with "compositor doesn't support the screen capture protocol". There is no
Xwayland either, so `import`/`xwd` are out. If `$DIR/env` isn't sourced (or the
session has expired) every tool silently aims at that display instead, which is
what those errors mean.

`swaymsg exec` runs anything else inside the session, which is also how to
measure viewer CPU: hide the window (switch workspace, or start a second engine
on another project to cover it), then diff utime+stime from `/proc/<pid>/stat`
(per thread under `task/`) over a few seconds. That is how the invisible-window
spin (2026-07-24) was found and fixed.

Input injection is `wdotool` on its wlr-protocols backend (libei wants a
RemoteDesktop portal the session doesn't have, so it auto-selects). Verified
2026-07-26 on wdotool 0.5.3 against sway: clicks (widgets and 3D pick), scrolls,
keys, drags and modifier-clicks all land. Two quirks:

- **Held state needs real time inside one call.** Button/modifier state dies with
  the `wdotool` process, so a drag has to be one chained invocation — but a chain
  runs in ~20ms, which is a single egui frame, and egui only sees a drag if the
  press survives a frame boundary. Pad the chain with a spacer that actually
  costs time: `type zzzzz` is ~12ms/char and the viewer ignores letters.

  ```sh
  G="type zzzzz"
  wdotool mousemove 400 200 $G mousedown 1 $G mousemove 500 250 $G mousemove 600 300 $G mouseup 1
  ```

  Orbit, middle-drag pan and shift-click-to-deselect all verified this way, as
  is dragging a panel edge — but grab a few px *inside* the panel: the
  viewport is registered after the panels, so on the edge pixel itself the
  orbit wins the drag.
  Do *not* use `click 8 --repeat 2 --delay N` as the spacer: it produces the same
  gap but the extra button breaks egui's drag tracking (it works fine for
  modifier-only holds).
- The first *vertical* scroll of a session is always swallowed. Throw one away,
  or warm up with a horizontal `scroll 1 0` — the viewer ignores dx.

`getmouselocation` and `getwindowgeometry` are unavailable on this backend (both
are send-only on Wayland); `search` / `getactivewindow` / `getwindowname` /
`getwindowclassname` / `outputs` all work.

The project dir must also be *short*: the engine's socket lives inside it and
a long path (the session scratchpad's, say) fails startup with "path must be
shorter than SUN_LEN". Copy the test project to `/tmp/<short>` instead.

Other limits: the fixed per-project socket path means parallel runs should use
different project dirs (`odm --project <dir> …` from outside the session reaches
an engine inside it fine — handy for checking `selection` after a click). An
in-process egui frame dump (`egui_kittest`, or an engine flag) would still be
the way to get deterministic UI snapshot *tests*; this is for looking, not
asserting.

## Remaining manual checks

In-window interaction has never had a human look: viewport feel
(orbit/pan/zoom), timeline scrub visuals, selection highlight. Everything else
in the MVP acceptance list was verified (fresh-checkout build, viewer launch on
Wayland, hot reload via CLI, tests), and clean exit on window close was fixed
(see "Owning the event loop") and verified.
