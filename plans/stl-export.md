# STL export

For 3D printing. Used mainly by the human from the viewer (File ▸ Export
STL…); the CLI command exists for agents and tests but is not the
design driver.

## Decisions (user, 2026-09-18)

- **Viewer exports the whole active tab**, always — no selection/per-node
  scope in the UI. To print one part of an assembly, open that part's
  doohickey in its own tab.
- **Export options** in the dialog (and as request fields): **Units**
  and **Union**. More can join them later.
- **Union, default on.** All solids in the scene are boolean-unioned in
  world space, so the file is one valid manifold: overlaps fuse,
  disjoint parts stay separate bodies. Off = every solid written as-is
  (exact, but overlapping siblings self-intersect).
- **`units` in `odm.toml`: `mm | m | in | ft`, default mm**, chosen
  with a selector in the New Project popup; export converts to mm
  (what slicers assume). The dialog's Units option starts at the
  project's unit and can be overridden per export. No free scale field.
- **Current inputs as shown** — panel values and current `t`. No preset
  picker.

Derived (not asked; cheap to revisit): color and opacity are ignored —
every solid in the scene exports, translucent "ghost" parts included
(see Open).

## Facts this rests on

- Every mesh in a scene is already a closed Manifold solid (open
  surfaces are rejected at `fromThreeGeometry`), so the only validity
  risk is overlapping siblings → union fixes it.
- `scene.rs::collect_meshes` already gathers a subtree's (mesh, world
  transform, label) triples for `clearance`. `Kernel::boolean` shows the
  n-ary world-space fold to copy; mirrored transforms go through
  `transformed_manifold` (Manifold flips winding on negative
  determinants).
- Kernel ops are **not** memoized (memo lives in the build layer) and
  `intern` puts an unrooted mesh in the store, which a concurrent gc may
  sweep before it is read back. So export does not go through
  `boolean`/store at all — see Core.
- manifold-csg 0.3.3 exposes `Manifold::decompose()` (body count).
- Scenes are Z-up right-handed like slicers: no reorientation. No
  recentering either (slicers place the part; 1:1 coordinates keep
  multi-file exports registered).
- `odm.toml` is `deny_unknown_fields` and not part of generation
  identity — right for `units`, which never affects a build. It is
  re-parsed by every `scan_project` (`sync.snapshot.marker`), so an
  edited unit is live at the next sync; the viewer dialog reads it fresh
  with `read_marker` at open (as `mod.rs` does for the name).

## Units

- `ProjectMarker` gains `units: Units` (`#[serde(default)]` = `mm`);
  enum `mm | m | in | ft`, each with a `to_mm()` factor. Unknown value
  = the usual BadMarker error listing the four.
- Existing projects have no key and read as mm (the bundled examples
  are mm-scale — hello-bracket says so — so nothing changes for them); the engine does
  not write it in (its only odm.toml write stays `engine`).
- File ▸ New Project (`viewer/new.rs`) gets a **Units selector** — mm
  (default), m, in, ft — and always writes `units` explicitly:
  `create_project(dir, name, units)` (callers: `viewer/mod.rs`, the
  `new_project` test suite). The starter block stays 40 × 30 × 20 mm in
  every unit: `starter()` takes the unit and emits `[40, 30, 20]`,
  `[0.04, 0.03, 0.02]`, `[1.5, 1.25, 0.75]` or `[0.15, 0.1, 0.06]`, with
  a `// units: <u> (odm.toml)` comment.
- Agents must know the unit to model in it: `status` reports `units`,
  `docs/api/determinism.md`'s "Units are yours" becomes "the project
  declares its unit in odm.toml (default mm); export relies on it", and
  the prompt gets one clause pointing at it. This is a deliberate
  prompt addition (wrong units = wrong prints, and the agent can't
  discover a convention it doesn't know exists) — record it in
  notes/agent-surface.md beside the `feedback` exception.
- No marker at all (a bare directory of .js) = mm.

## Core: `odm-kernel` + a small writer

- `StlOptions { units: Units, union: bool }` — one struct shared by the
  request and the dialog, so a new option is added in one place. `Units`
  lives in odm-build (with the marker); the kernel takes a plain scale.
- Kernel: `export_solids(operands: &[(Hash, Transform)], union: bool,
  cancel) -> Result<Vec<ExportSolid>>`, `ExportSolid { mesh: Mesh (f64),
  volume, bodies }`. Works on Manifolds directly and returns the meshes
  — **nothing is put in the store**, so no gc gate and no garbage.
  Union on: fold like `boolean`, one result. Off: one per operand, in
  order. Each goes through `evaluated(m, cancel)` (status check +
  cancellation) like `intern`; `bodies` = `decompose().len()`. The
  caller folds the unit scale into the transforms, so volume and
  positions come back in mm.
- Inputs must stay alive while this runs: the CLI path holds
  `query_view`'s `RootPin`; the viewer takes `store.pin_root` on the UI
  thread at dialog open (the tab's `Published` still holds the root
  then) and moves the pin into the worker.
- `odm-engine/src/stl.rs`: `export_stl(store, kernel, root: &Node,
  header: &str, out: &Path, opts: &StlOptions, cancel) ->
  Result<StlReport, String>`:
  1. `collect_meshes` from the root → operands (labels unused); none →
     error "nothing to export (the scene has no solids)".
  2. Scale by `opts.units.to_mm()` (world transform premultiplied),
     `kernel.export_solids`.
  3. Write **binary STL**: 80-byte header (`ODM <project> <view path>`,
     non-ASCII → `?`, truncated/zero-padded, never starting with
     `solid`, no timestamp → deterministic bytes), u32 count,
     per-triangle f32 normal + 3 f32 vertices + 0 attribute. Normals
     computed from the f64 positions before narrowing (zero-area → 0 0
     0; slicers recompute). No ASCII variant. More than u32::MAX
     triangles → error.
  4. Drop only triangles where two vertices narrow to the **same f32
     point** (a collapsed edge takes both its triangles, so the surface
     stays closed); keep collinear slivers — dropping those would open a
     hole. Count the drops; warn if any.
- `StlReport { path, units, union, size_mm: [f64;3], volume_mm3, tris,
  bodies, warnings }` (options echoed as resolved; with union off,
  `volume_mm3` and `bodies` are sums over the solids). Warnings:
  `bodies > 1` ("N separate bodies — they will print as loose parts";
  union on only — with union off and more than one solid, "N solids
  written as-is; overlaps are not fused" instead), longest side < 1 mm or > 2000 mm ("check
  `units` in odm.toml"), dropped triangles. The size thresholds are one
  shared fn, used by the dialog's live line too.
- Write atomically (temp file beside the target + rename) — the target
  may be open in a slicer that watches it.

## Engine command

`odm export '{"out": "part.stl", …view fields}'` — a normal spec-table
entry in `requests.rs` (path/inputs/preset/view + `out`, required,
plus the options: `units` — default the project's — and `union` —
default true).
Format comes from the `out` extension; only `.stl` today, anything else
errors listing supported formats (leaves room for 3MF, which carries
units/colors/multiple objects and is the likely follow-up). Response =
`StlReport`. `out` resolves client-side against the CLI's cwd (the CLI
already does this for any command's `out`) and passes `check_out_path`
like render's. Flow: `query_view` (sync + build, keep the `RootPin`) →
`export_stl` gate-free, like the other post-build reads.

`odm export --web …` stays the standalone path: `crates/odm/src/main.rs`
today sends every `export` to `export_site`; route there only when the
first arg after `export` starts with `--` (or is absent → its existing
usage error, now mentioning both forms), else to `odm_cli::run`. `odm --help` gets one line; `docs/cli.md` a
short "Exporting for printing" section. Docs-only otherwise.

No `node` field in the first cut (the UI has no per-node scope and the
user doesn't need it); `collect_meshes` makes it a small later
addition.

## Viewer: File ▸ Export STL…

- `menu.rs`: new `Action::ExportStl` under Export Web…; disabled when
  there is no active tab or its last build has no result.
- `viewer/export_stl.rs`, modelled on `export.rs` (same `Browser`,
  fixed-width `theme::dialog`, Pick → Running → Done stages):
  - **Snapshot at open**: the dialog captures the tab's
    `Published { view, root }` (+ pin, + `tab.scene` bounds) when it
    opens and exports exactly that, even if the tab keeps playing or
    rebuilding behind it — header, size line and file always agree.
  - Header line: `Exporting <view path>` + the snapshot view's set
    inputs in the caption form `frames` uses (`t=0.75`), so it's clear
    which instant is captured. If `published.building` was set at open,
    add a weak "a newer build is pending — exporting what is shown".
  - **Options** block under the browser: `Units: [mm ▾]` (the four
    units; starts at the project's, labelled e.g. "mm (project)") and
    `[x] Union overlapping solids`. Under them a live size line,
    `0.08 × 0.06 × 0.0042 m → 80 × 60 × 4.2 mm`, from the scene bounds
    × the chosen unit (no kernel work), turning warning-colored when
    the longest side is < 1 mm or > 2000 mm — the wrong-unit catch,
    before export rather than after. Choices last for the session;
    each new session starts from the project unit and union on.
  - Browser starts where the last STL went this session, else the
    project dir. "File:" defaults to `<doohickey stem>.stl`;
    overwrite asks once inline.
  - Runs on a worker thread like `export.rs` (channel +
    `request_repaint`). It needs the kernel and store
    (`build_engine()`), no JS and no build gate: the snapshot root, not
    a fresh build — exactly "what you see", never blocks on a build in
    flight, never touches `JsEnv`. Dismissing mid-export cancels the
    token; the temp file is removed.
  - Done stage shows the report: `80 × 60 × 4.2 mm · 1 body · 2,312
    triangles`, warnings in the warning color, and the path (click to
    copy, like the web export's).
- STL files inside the project are inert (scanner only takes `.js`), so
  no marker/destination checks are needed.

## Tests

- Writer unit tests: byte-exact golden for a cube (header, count,
  normals outward, 50 bytes/tri), determinism (two exports identical),
  unit scaling (cube of 0.02 in `m` → 20 mm; same cube as `mm` → 0.02 mm + size warning; `ft` → 6.096 mm).
- Union: two overlapping cubes → one body, volume = union volume;
  two disjoint → `bodies: 2` + warning; mirrored instance → positive
  volume when re-read. Store object count unchanged by an export.
- Narrowing: a mesh with an edge shorter than f32 resolution at its
  offset → both triangles dropped, round-trip still watertight.
- Round-trip: parse the written STL back, weld, `solid_from_mesh`
  accepts it (proves watertight after f32 narrowing).
- Options: `units` override beats the project's; union off → two
  overlapping cubes keep both shells (tri count = sum), report echoes
  the resolved options.
- Marker: `units` absent = mm, bad value error text, `status` echo.
- New Project: selector default mm; chosen unit lands in odm.toml.
- Command: spec-table entry, unknown extension error, `out` onto a
  project `.js` refused, `export --web` still routes to odm-export.
- Dialog: headless-egui test in the style of the existing dialog tests
  (default filename, stale notice, report line).

## Order

1. `units` in the marker + status/docs/prompt/New Project.
2. Kernel `export_solids`, `stl.rs` writer + report, tests.
3. `export` engine command + CLI routing + docs.
4. Viewer dialog + menu item.
5. Notes: architecture.md (crate map, menu list, project format),
   agent-surface.md (prompt exception, `export` command); delete this
   plan.

## Open / later

- 3MF (units, colors, per-part objects) via the same command and a
  format choice in the dialog.
- Per-node export (`node` field; tree context menu) if whole-tab proves
  too coarse for assemblies.
- Ghost parts: if translucent reference geometry fusing into prints
  bites, add an option (or skip solids below some effective alpha).
- Showing the unit in the viewer (grid label, inspect sizes) now that
  the project declares one.
