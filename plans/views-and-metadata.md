# Plan: views, unified inputs, doohickey metadata

Decisions from design discussion with user (2026-07-29, marker revised
2026-07-29). Replaces main.js, odm.json, `ctx.param`, `ctx.t`, `ctx.args`,
and `animation.duration`. No back-compat — pre-v1, break freely.
Multi-agent (chat identity, build fairness) is explicitly deferred; views
make it possible later.

## Model (decided)

- **View** = (doohickey path, args, cascade values), evaluated against the
  current generation. Any `.js` file is viewable/queryable; a viewer tab
  holds one view; every CLI query targets one (default: `root.js` with
  declared defaults, when that file exists). Views stay pure functions of
  (generation, inputs) — the byte-reproducibility invariant is untouched.
- **odm.toml** replaces main.js/odm.json as the project marker for the
  walk-up. Contents: `name` (project name, shown in window title/status)
  and `engine` (last-used *engine* version — an integer, currently 0,
  independent of the per-file JS API version). On open, the engine warns
  if the file's `engine` is newer than itself (project last touched by a
  newer engine), and writes its own version back when it differs — the
  ONE exception to "the engine never writes project files". Unknown keys
  rejected. Not part of generation identity: it never affects build
  output, and the engine writing it must not churn generations. Authored
  at project creation (by hand/agent; no init flow exists — a future
  `odm init` is a nicety, not part of this plan).
- **root.js is pure convention**: the entry-file name the CLI/viewer try
  by default when no path is given, like `index.html` — used if present,
  nothing structural, and projects are free to name entry files
  meaningfully instead. No path and no root.js: CLI queries error listing
  viewable files; the viewer restores tabs from `.odm/` or shows a
  picker. Permanent knob changes = editing defaults in code (agent's
  job).
- **`.odm/`** stays engine-private local state (socket, tab persistence,
  caches) — never user-authored, never a project marker.
- **Doohickey metadata**, two parts:
  - Prose description in the `//!` comment block (shared with the version
    pragma, already parsed at sync time in
    odm-build sources.rs WITHOUT evaluating the module — survives broken
    builds, greppable). First line = one-sentence summary, rest = body.
  - `export const meta = { inputs: {...}, presets: {...} }` — structured,
    read by evaluating the module (no build), cached by code hash.
- **Inputs**: `meta.inputs` = ONE map, name → entry; an entry is a
  profiled JSON Schema plus ODM keys. One namespace by construction.
  - Default kind: from the immediate caller only (the view panel/CLI when
    this file is a root). No `default` = required.
  - `cascade: true`: nearest provider up the invoke chain, view
    outermost; `default` mandatory.
  - Access is uniform: `ctx.get(name)` — the declaration decides
    resolution. Reading an undeclared name = error; unknown keys in an
    invoke's args = error.
  - Schemas: strict JSON Schema profile — `type`, `enum`, `default`,
    `description`, `minimum`/`maximum`, `items`, `properties`/`required`.
    Unknown keywords rejected (deny_unknown_fields philosophy) via our own
    allowlist walk at meta extraction — which is also where ODM keys
    (`cascade`) are recognized, so a typo'd key errors instead of silently
    changing an input's kind. ODM keys are stripped and extension types
    desugared before value validation, which is delegated to the
    `jsonschema` crate (MIT — license ok; user approved the dep; spec
    ignores unknown keywords, so ODM keys are tolerated anyway). Engine
    validates values at invoke and view boundaries. `default` has real
    semantics (documented deviation, as in OpenAPI).
  - ODM extension types: `'solid'`, `'vector2'`, `'vector3'`,
    `'quaternion'`, `'matrix4'`, `'color'`. Declaration-driven hydration:
    on the wire they are canonical JSON (vectors `[x,y,z]`, matrix4 = 16
    numbers column-major = three's `toArray` order, color = CSS string or
    `[r,g,b]`) so hashing/memoization is unchanged, but `ctx.get` returns
    real vendored-THREE instances (Solid stays a Solid handle). Senders
    (invoke args, provides, meta defaults) may pass THREE instances or the
    JSON form; normalized at the boundary. For crate validation each
    extension type desugars to a standard schema (e.g. vector3 →
    fixed-length number array); `solid` is checked as our handle tag.
    Extension types also drive typed viewer controls (vector row, color
    picker).
- **Resolution rule (cascade)**: nearest explicit provide above the reader
  (view outermost); else the default from the *shallowest* declaration on
  the reader's own invoke path, including itself. I.e. declaring a cascade
  input auto-provides its default for your subtree when nothing above
  covers the name — so any subtree under a common declaring ancestor
  agrees on the value whether or not it was explicitly set. Resolution
  depends only on the reader's invoke path (memo-friendly; it is just a
  provide). Post-build lint: same name falling through in unrelated
  subtrees with conflicting defaults → warning; conflicting types → error.
- **Invoke**: `ctx.invoke(path, args?, provides?)` — both optional.
  Provides need no declaration (may target descendants the provider does
  not know) and scope over the whole subtree. Args and provides are
  separate channels: cascade inputs cannot be passed via the args object.
- **Time is convention, not a feature**: no animation/duration concept
  anywhere in the engine. `t` is a cascade number with a range. The viewer
  renders a numeric, ranged fall-through control named `t` as a transport
  (scrub + play at 1 unit/sec, looping over the range); everything else
  is a plain generated control. CLI sets any input via `--set name=value`;
  `--t` is gone. A wheel declaring t 0–2 IS "this loops every 2 seconds",
  standalone or composed.
- **Presets**: `meta.presets = { name: {input: value, ...} }` — named
  input bundles (both kinds). One click in the viewer, `--preset name` in
  the CLI, future test fixtures. The Storybook "stories" idea.
- **Viewer/CLI state contract**: viewer-edited view state (inputs, camera,
  selection, per tab) lives in memory, persisted in `.odm/`, never in
  project files, and never affects CLI answers by default. CLI = declared
  defaults + explicit `--set`/`--preset` (deterministic, code-truth).
  Opt-in: `--view <tab>` / `--viewer-state` adopts a tab's state. `odm
  poll` messages carry a snapshot of the user's active view (path, input
  values, selection) — generalizes the selection query; the main way
  viewer state reaches the agent.

## Phases

### 1. Metadata (additive, lands first)
- extend the `//!` block parser in sources.rs (version pragma already
  landed) to also capture the description; stored on the source record.
- meta extraction: evaluate module, validate meta shape + schema profile
  (allowlist walk + extension-type desugaring; add `jsonschema` crate),
  cache by code hash.
- `odm interface <path>`: description, input schemas, presets.

### 2. Unified inputs (the invasive engine phase)
- Framework: `ctx.get`, new `ctx.invoke` signature; delete `ctx.t`,
  `ctx.param`, `ctx.args`. Boundary normalization + hydration for the
  extension types (THREE instances ↔ canonical JSON).
- odm-build: context becomes a per-invoke-path environment (explicit
  provides + auto-provided declaration defaults), not per-pass globals.
  Memo: `Dep::Invoke` records provides alongside args; `Dep::Context`
  validates against the reader's environment. Pass = generation + view
  (root path, args, view-level provides).
- Schema validation at boundaries; fall-through + conflict lint recording.
- Delete odm.json (params/animation plumbing throughout). Marker swap:
  `find_project`/`is_project` (odm-cli lib.rs, odm-engine session.rs,
  viewer open.rs) look for odm.toml only; add `toml` crate (MIT/Apache);
  parse name+engine, reject unknown keys; newer-engine warning + engine
  writes back its version on open (only when it differs). main.js loses
  all special status — `ROOT_DOOHICKEY` becomes "root.js if present"
  until phase 3 deletes the fixed-root concept; examples get an odm.toml
  and a root.js entry file. Regenerate IR-hash goldens; rewrite docs/api
  (params-and-animation.md → inputs.md; touch doohickeys/composition,
  project-format docs) and docs/prompts; amend the "engine never writes
  project files" invariant (CLAUDE.md, notes/architecture.md) with the
  odm.toml exception.

### 3. View plumbing: engine + CLI
- EngineState: published slot → map keyed by view; background loop builds
  active views (per-view latest-wins).
- Per-build fall-through report: names resolved at view level or by
  auto-provide, with winning declarations (shallowest wins default/range;
  equal-depth ranges union; type conflict = error) — the input panel's
  data source.
- CLI: queries take optional path + `--set`/`--preset`; default target =
  root.js if present (convention only), else error listing viewable
  files. tree/inspect/raycast/render/selection become view-scoped;
  status `has_root` → "default view target exists".
- Note: issues/engine-serializes-commands.md bites harder with multiple
  views; not addressed here.

### 4. Viewer: tabs + input panel
- Tabs (no multi-window), one view each: path, input state, camera,
  selection, tree state. Persisted in `.odm/`.
- Input panel generated from root args + fall-through report; `t`
  transport treatment (replaces the duration-keyed timeline); typed
  controls per extension type (vector fields, color picker); presets.

### 5. Viewer↔CLI bridge
- `--view <tab>` / `--viewer-state` on queries; poll messages carry the
  active-view snapshot.

Order: 1 first. 2 is the big one. 3 before 4 (CLI targeting is useful
before tabs exist). 5 last.
