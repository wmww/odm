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
  Validation at every boundary via the `jsonschema` crate; unknown
  args/schema keys/input names are errors.
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
  CLAUDE.md as a relative symlink to it. Anything else is a viewer question,
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
  memo cache (key = code+args hashes; entry = recorded deps + output;
  `Dep::Invoke` stores the actual args Value so validation can re-run
  invokes, `Dep::Cascade` a value hash — missing keys hash a sentinel, use
  `odm_js::cascade_value_hash`; eviction is whole-cache clear only, see
  issues/memo-cache-policy.md).
- `odm-kernel` — manifold-csg wrapper: primitives (cylinder along Z),
  extrude/revolve (around Z), booleans/hull with per-operand transforms,
  weld with boundary-edge diagnosis (Manifold's own error is bare
  NotManifold), raycast (Manifold returns distance as a *fraction* of the
  segment; kernel converts), clearance (exact overlap + AABB gap lower
  bound; see notes/agent-surface.md), volume/area/bounds, CancelToken
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
- `odm-js` — deno_core =0.408.0; per-build disposable isolates from ONE
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
- `odm-build` — one `BuildEngine` per project; `sync()` rescans it and
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
  names, each entry carrying `kind`), winning declarations, lints
  (conflicting defaults/types, unread cascade values, plain-shadows-cascade)
  — the input panel's data source and the input-name typo check
  (`check_input_names`); `declared_entries` is the failure-path subset
  (target's declared schema, no walk needed). Each pass also collects
  `BuildStats` (per-doohickey runs + self-time, memo hits) into
  `PassResult.stats`, surfaced by `"stats": true` on any view command.
  In-flight registry (wait-for-in-flight + wait-graph cycle detection);
  cancellation (token + TerminateExecution post-module-eval). Cycle check is
  keyed on (path, args-hash, env-hash) so bounded recursion works; memo
  entries carry console logs and replay them on hits. Passes run
  concurrently across threads (each CLI connection + the build loop); the
  registry's cross-thread dedup/cycle machinery is what makes that safe
  (`concurrent_same_pass_dedups`, `commands_overlap_an_in_flight_build`).
  Within one pass `get_or_build` still recurses inline.
- `odm-render` — wgpu =29.0.4 (MUST track egui's pinned wgpu major);
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
- `odm-engine` — library, entered via `odm run` (`run_headless(project)` /
  `run_viewer(Option<project>)`).
  Headless: socket server only. Default: + eframe
  viewer (menu bar, tab strip — one view per tab, persisted in
  `.odm/viewer.json`; classic notebook tabs, each with its own close box, a
  red label when that tab's last build failed —, offscreen texture viewport via
  register_native_texture, orbit/pan/zoom, tree panel, generated input panel
  (right side; controls from the tab's fall-through report: trackbars for
  ranged numbers, toggles, choice buttons, JSON-ish text fields, presets), a
  `t` transport when a ranged cascade number named t falls through
  (scrub + play at 1 unit/sec looping), error + console panels with
  last-good scene (`Published.logs` is latest-attempt: success or failure,
  colored by level), click-select via CPU raycast when shaded / nearest-wire
  screen-space pick when wireframe, shift-click to select several),
  background build loop over the active view slots
  (per-slot latest-wins, Pass::cancel on same-slot supersede; a new
  generation re-queues every slot), notify-based watcher (150ms
  debounce; its dot-dir filter applies to the path *relative to the project*,
  since the project itself may live under one). The viewer never polls: it
  repaints when `EngineState::wake` fires (set by the viewer, unset when
  headless), i.e. on every `published` change. It also owns its winit event
  loop so `SlowIdle` can fix up what eframe leaves behind — see viewer/idle.rs
  and "Owning the event loop" below.
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
  a one-off build's root is otherwise pinned only by its memo entry, and a
  concurrent rebuild of the same (code, args) under different cascade
  values — a viewer scrub — overwrites that entry, after which a publish's
  gc would sweep the scene mid-read. (Published slot roots are pinned as
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
  `watcher.rs`, `server.rs`, `session.rs`, `viewer/` (`mod.rs` app + viewport,
  `idle.rs` event loop, `menu.rs` menu bar, `browse.rs` folder list with
  `open.rs`/`new.rs` on top of it, `tree.rs` scene tree, `tabs.rs` tab state,
  `inputs.rs` input panel — see "Input panel" below), `scene.rs`, `theme/`,
  `icons.rs`.
  `theme/` holds the viewer's dark Windows 95
  look (classic bevel structure, inverted luminance, white text):
  a `Style`/`Visuals` preset plus widget wrappers (`button`,
  `collapsing`, `list_box`, `text_edit`, `trackbar`, `menu_bar`/`menu`,
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
  never an ancestor); everything else → `odm_cli::run`. Top-level `--help` splices in
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

`viewer/inputs.rs` + `viewer/tabs.rs`. Controls come from the tab's
fall-through report; interactions come back as `Event`s which
`inputs::apply` folds into the tab's `set_args`/`set_cascade` (pure and
unit-tested; `mod.rs` then submits `tab.view()` to the engine). The
invariant that keeps it honest: **the panel is a pure render of (report, tab
set values)** — the only other state is `Tab::edit`, the buffer of the one
text field currently holding keyboard focus (egui focus is single, so it's
an `Option`, present exactly while focused; Enter applies, any other focus
loss discards). Two rules that came out of a 2026-08-14 bug (a text field
frozen at its first-frame value, blind to presets/×):

- Never cache what a control displays outside `Tab::edit`. Unfocused text
  fields re-derive their string from `shown_value` every frame.
- `shown_value` = the tab's set value, else the entry's declared *default* —
  never the report's resolved `value`, which is from the last successful
  build and lags the set values (briefly after any change, forever if the
  build fails). The tab is the only writer of view-level values, so unset
  always means "resolves to the default".

Text fields get stable explicit egui ids (`("input", section, name)` via
`theme::text_edit`'s id param) so focus/cursor state survives rows
appearing above them — and so tests can find them. `inputs.rs` tests drive
the real panel through a headless `egui::Context` (real clicks on painted
labels, real focus, asserting on the painted galley text).

### Menu bar, and switching projects

`viewer/menu.rs` is the whole bar: an `Action` enum, a `theme::menu` per
drop-down listing `MenuEntry`s, and one `apply` that turns an action into an
effect. File has New Project…/Open Project…/Exit, View has Frame Scene (F) and checkmarked
Wireframe/Grid. `theme::menu` measures its own entries and pins the popup width
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

- **Poll's contract is set by agent harnesses.** The baseline one can't read a
  running background command's output — it is woken when the command *exits*.
  So poll blocks until ≥1 message is queued, prints them all, and exits;
  process exit is the delivery mechanism. `--timeout` bounds the wait (empty
  `messages`), and a retired session (File ▸ Open) answers `{"ok": false,
  "error": {"kind": "stopped"}}` so a poll never outlives its engine.
- **`--follow` is the same thing for harnesses that watch lines** (Claude
  Code's Monitor, say): park one command at session start and every batch
  arrives as a push — no relaunch per message, and no gap where nobody is
  listening. Purely a CLI-side loop (`follow_poll`): send poll, print the
  response as one compact JSON line, flush, ack, poll again — so the engine and
  the protocol's one-response-per-request rule are untouched, and delivery is
  the same two-phase handshake per batch. A `--timeout` alongside it is an
  error (nothing to bound). `docs/prompts/cli.md` tells the agent to park a
  follow if its harness can watch lines, and to loop `--timeout` otherwise.
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
- Viewer: a fixed-height panel above the status band —
  `theme::tail_box` transcript (user lines `> …` white, dimmed while
  undelivered; agent lines in `theme::AGENT_TEXT`) plus one `theme::text_edit`
  where Enter sends and keeps focus. The status band says whether the agent is
  listening, which is the user's cue to go prod it in its own terminal. All of
  it repaints through the existing `EngineState::wake`.

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
  File ▸ Exit sets the same shared `Quit` flag; `about_to_wait` acts on it.
  Both paths verified to reach `process::exit(0)`.

### Viewer fonts

`crates/odm-engine/assets/fonts/` holds two bitmap faces converted from X11
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

`crates/odm-engine/assets/icons/` — one 11×11 RGBA PNG per icon,
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
`viewer/tree.rs` walks the tree telling it where each row sits (depth, which
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
  viewer/tree.rs, not egui's `CollapsingState`. The box toggles, the name selects,
  a double-click on the name does both.
- Selecting a node auto-expands its ancestors, and collapsing them again when
  the selection goes away is why the state is ours: `TreeState::auto` remembers
  what each auto-expand displaced, and any user toggle (`set_manual`) takes
  that node out of auto-expand's hands for good. Nodes above `AUTO_DEPTH`
  start open.
- Selection is a list, in pick order. Shift-clicking a row — or a solid in the
  viewport — adds it, or removes it if it was already selected; a plain click
  replaces the whole selection. `status` (active slot) and poll
  snapshots report the list.
- `viewer::tree::tests` drives rows through a headless `egui::Context` (real hit
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
  files that are not there.)
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
`odm-prompt/src/` (marker splicing + the agent-file scan) and, in
odm-engine, `icons.rs`, `commands.rs`, `state.rs` (the chat queue),
`server.rs` (delivery over a real socket), `viewer/tree.rs`,
`viewer/inputs.rs` (the input panel, headless egui), `theme/scroll.rs`, and
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
  (`cargo test -p odm-build --test doctests`; ` ```js skip` opts out).
  Keep docs examples self-contained — free variables fail the build.

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

  Orbit, middle-drag pan and shift-click-to-deselect all verified this way.
  Do *not* use `click 8 --repeat 2 --delay N` as the spacer: it produces the same
  gap but the extra button breaks egui's drag tracking (it works fine for
  modifier-only holds).
- The first *vertical* scroll of a session is always swallowed. Throw one away,
  or warm up with a horizontal `scroll 1 0` — the viewer ignores dx.

`getmouselocation` and `getwindowgeometry` are unavailable on this backend (both
are send-only on Wayland); `search` / `getactivewindow` / `getwindowname` /
`getwindowclassname` / `outputs` all work.

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
