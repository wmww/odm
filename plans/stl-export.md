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
- **`units` in `odm.toml`, default metres**; export converts to mm
  (what slicers assume). The dialog's Units option starts at the
  project's unit and can be overridden per export. No free scale field.
- **Current inputs as shown** — panel values and current `t`. No preset
  picker.

## Facts this rests on

- Every mesh in a scene is already a closed Manifold solid (open
  surfaces are rejected at `fromThreeGeometry`), so the only validity
  risk is overlapping siblings → union fixes it.
- `Kernel::boolean(Union, &[(Hash, Transform)])` is n-ary and
  world-space; `scene.rs::collect_meshes` already gathers a subtree's
  (mesh, world transform) pairs for `clearance`. Mirrored transforms go
  through `transformed_manifold`, so winding is handled.
- Scenes are Z-up right-handed like slicers: no reorientation. No
  recentering either (slicers place the part; 1:1 coordinates keep
  multi-file exports registered).
- `odm.toml` is `deny_unknown_fields` and not part of generation
  identity — right for `units`, which never affects a build.

## Units

- `ProjectMarker` gains `units: Units` (`#[serde(default)]` = `m`);
  enum `mm | cm | m | in`, each with a `to_mm()` factor. Unknown value
  = the usual BadMarker error listing the four.
- Existing projects have no key and read as **metres**; the engine does
  not write it in (its only odm.toml write stays `engine`). The bundled
  examples are all modelled in mm — give each `units = "mm"` (and check
  any project templates/tests that assume mm-scale numbers).
- File ▸ New Project writes `units` explicitly; add a units dropdown to
  the dialog (default m).
- Agents must know the unit to model in it: `status` reports `units`,
  `docs/api/determinism.md`'s "Units are yours" becomes "the project
  declares its unit in odm.toml (default m); export relies on it", and
  the prompt gets one clause pointing at it. This is a deliberate
  prompt addition (wrong units = wrong prints, and the agent can't
  discover a convention it doesn't know exists) — record it in
  notes/agent-surface.md beside the `feedback` exception.
- Unit changes are picked up like any marker change (at sync); confirm
  the marker is re-read on sync, not only at open.

## Core: `odm-kernel` + a small writer

- `StlOptions { units: Units, union: bool }` — one struct shared by the
  request and the dialog, so a new option is added in one place.
- `odm-engine/src/stl.rs` (or `scene.rs` neighbour): `export_stl(store,
  kernel, root: &Node, opts: &StlOptions, cancel) -> Result<StlReport>`:
  1. `collect_meshes` from the root → operands; none → error "nothing
     to export (the scene has no solids)".
  2. Fold `opts.units.to_mm()` into each operand's transform. Union on:
     `kernel.boolean(Union, …)` (content-addressed, so a re-export is a
     cache hit). Union off: `transform_solid` per operand, triangles
     concatenated in tree order.
  3. Write the resulting mesh(es) as **binary STL**: 80-byte header
     (`ODM <project> <view path>`, never starting with `solid`, no
     timestamp → deterministic bytes), u32 count, per-triangle f32
     normal + 3 f32 vertices + 0 attribute. Normals computed from the
     f64 positions before narrowing. No ASCII variant.
  4. Skip triangles that become degenerate after f32 narrowing (count
     them; warn if any).
- New kernel query `bodies(h) -> usize` (Manifold `decompose` count, or
  a union-find over the indexed mesh — whichever is cheaper to wire).
- `StlReport { path, units, union, size_mm: [f64;3], volume_mm3, tris,
  bodies, warnings }` (options echoed as resolved; with union off,
  `volume_mm3` is the per-solid sum and `bodies` counts per solid).
  Warnings: `bodies > 1` ("N separate bodies — they will
  print as loose parts"), longest side < 1 mm or > 2000 mm ("check
  `units` in odm.toml"), dropped degenerate triangles.
- Write atomically (temp file + rename) — the target may be open in a
  slicer that watches it.

## Engine command

`odm export '{"out": "part.stl", …view fields}'` — a normal spec-table
entry in `requests.rs` (path/inputs/preset/view + `out`, required,
plus the options: `units` — default the project's — and `union` —
default true).
Format comes from the `out` extension; only `.stl` today, anything else
errors listing supported formats (leaves room for 3MF, which carries
units/colors/multiple objects and is the likely follow-up). Response =
`StlReport`. `out` resolves client-side against the CLI's cwd like
`render`'s.

`odm export --web …` stays the standalone path: `crates/odm/src/main.rs`
routes `export` to odm-export only when the first arg is a `--flag`,
else to `odm_cli::run`. `odm --help` gets one line; `docs/cli.md` a
short "Exporting for printing" section. Docs-only otherwise.

No `node` field in the first cut (the UI has no per-node scope and the
user doesn't need it); `collect_meshes` makes it a small later
addition.

## Viewer: File ▸ Export STL…

- `menu.rs`: new `Action::ExportStl` under Export Web…; disabled when
  there is no active tab or its last build has no result.
- `viewer/export_stl.rs`, modelled on `export.rs` (same `Browser`,
  fixed-width `theme::dialog`, Pick → Running → Done stages):
  - Header line: `Exporting <view path>` + the tab's current inputs in
    the caption form `frames` uses (`t=0.75`), so it's clear which
    instant is captured.
  - **Options** block under the browser: `Units: [m ▾]` (the four
    units; starts at the project's, labelled e.g. "m (project)") and
    `[x] Union overlapping solids`. Under them a live size line,
    `0.08 × 0.06 × 0.0042 m → 80 × 60 × 4.2 mm`, from the scene bounds
    × the chosen unit (no kernel work), turning warning-colored when
    the longest side is < 1 mm or > 2000 mm — the wrong-unit catch,
    before export rather than after. Choices last for the session;
    each new session starts from the project unit and union on.
  - Browser starts where the last STL went this session, else the
    project dir. "File:" defaults to `<doohickey stem>.stl`;
    overwrite asks once inline.
  - Runs off the UI thread. It needs the kernel but no JS: export the
    tab's **last published result** (the node hash the viewport is
    showing), not a fresh build — that is exactly "what you see", never
    blocks on a build in flight, and avoids touching `JsEnv`. If the
    tab is stale (newer build pending), say so in the dialog and let
    the user export anyway.
  - Done stage shows the report: `80 × 60 × 4.2 mm · 1 body · 2,312
    triangles`, warnings in the warning color, and the path (click to
    copy, like the web export's).
- STL files inside the project are inert (scanner only takes `.js`), so
  no marker/destination checks are needed.

## Tests

- Writer unit tests: byte-exact golden for a cube (header, count,
  normals outward, 50 bytes/tri), determinism (two exports identical),
  unit scaling (cube of 0.02 in `m` → 20 mm; same cube as `mm` → 0.02 mm + size warning).
- Union: two overlapping cubes → one body, volume = union volume;
  two disjoint → `bodies: 2` + warning; mirrored instance → positive
  volume when re-read.
- Round-trip: parse the written STL back, weld, `solid_from_mesh`
  accepts it (proves watertight after f32 narrowing).
- Options: `units` override beats the project's; union off → two
  overlapping cubes keep both shells (tri count = sum), report echoes
  the resolved options.
- Marker: `units` absent = m, bad value error text, `status` echo;
  examples still export at mm scale.
- Command: spec-table entry, unknown extension error, `export --web`
  still routes to odm-export.
- Dialog: headless-egui test in the style of the existing dialog tests
  (default filename, stale notice, report line).

## Order

1. `units` in the marker + status/docs/prompt/New Project.
2. Kernel `bodies`, `stl.rs` writer + report, tests.
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
- Showing the unit in the viewer (grid label, inspect sizes) now that
  the project declares one.
