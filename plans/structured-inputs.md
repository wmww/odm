# Structured inputs

User-editable structure in view inputs: arrays you can grow ("new
object"), objects with typed properties, nested to arbitrary depth, all
rendered as real controls in the input panel instead of one JSON text
field. Decided 2026-08-17 with the user: structure lives **in the input
values** (option B), not in a selection-contextual panel (option A) —
the data model is the view's `(args, cascade)` either way, selection
contextuality would need output→input provenance the pure build cannot
provide, and the CLI/agent must see the same structure as the panel.
Selection↔panel linking can be layered on later (see Follow-ups).

The headline scenario: a scene doohickey declares

```js
objects: { type: 'array', default: [], items: { type: 'object',
  properties: {
    position: { type: 'vector3', default: [0, 0, 0] },
    kind: { enum: ['box', 'sphere'], default: 'box' },
  } } }
```

and the panel shows an Add button, one group per element, and a typed
row per property. `build()` maps elements through
`ctx.invoke('parts/object.js', obj)` so editing one object memo-hits
the rest.

## Design rules

- **Arbitrary depth everywhere.** Every mechanism below is a recursive
  walk over the schema tree, never a special case for "one level down".
  The leaf grammar at depth N is exactly the top-level grammar today
  (minus `cascade`, which stays top-level-only — it names a resolution
  channel, not a shape).
- **View identity is untouched.** A structured value is still one JSON
  value pinned under one input name. Pin/reset/`set_or_clear`/
  persistence/CLI all keep operating on whole top-level values;
  `same_value` already compares deep. `{"inputs": {"objects": [...]}}`
  works on the wire today and stays the agent's surface.
- **The panel invariant survives**: still a pure render of (report, tab
  set values); the only new state is that `Tab::edit` keys on a *path*,
  not just a name.

## 1. Schema layer (odm-build `meta.rs`, framework hydration)

- **Extension types at any depth.** Drop the "only supported at the top
  level of an input" restriction for vector2/3, quaternion, matrix4,
  color. `solid` stays top-level-only: it is an opaque handle with no
  default and no authorable value, so it has no place inside panel-built
  JSON.
- **Recursive desugar.** The validator is built by walking the schema
  and replacing each extension-typed node with its desugared standard
  schema (today's `ExtType::desugar`, applied at every depth).
- **Recursive normalize.** `Input::accept` walks value and schema
  together, applying `ExtType::normalize` at each extension-typed
  position (inside array items, object properties, recursively) so
  THREE-instance forms canonicalize at any depth before hashing.
- **Nested `default`s, applied.** Allow `default` in nested schemas.
  Doctrine (docs/api/inputs.md): "`default` has real semantics — it is
  applied, not just documented" — so nested defaults are *applied at
  normalization*, not panel-only: `accept` fills an absent object
  property that declares a default (recursively), so the build, memo
  identity, presets, and the panel all see the same filled value.
  Array `items.default` is different in kind — absent elements don't
  exist — it is the **new-element template** (see §4). No existing
  project can have nested defaults (they were rejected), so no golden
  or memo-identity compat concerns.
- **Recursive hydration.** `ctx.input` walks the declared schema and
  hydrates every extension-typed position into THREE instances (today:
  top level only). Same walk as normalize, on the JS side.
- Conformance tests (`tests/conformance/unstable/`) for: nested ext
  type round-trip, nested default fill, nested validation errors
  naming the path, deep normalize of THREE-instance forms.

## 2. Report carries the schema (odm-build `report.rs`)

`ReportEntry` currently flattens to `ty/minimum/maximum/choices/
default` — `items`/`properties` are dropped, so the panel cannot see
structure. Add `schema: Map<String, Value>`: the winning declaration's
authored schema (nested defaults normalized), arbitrary depth for free
since it is the authored JSON. Keep the flat convenience fields
(derive them from the schema rather than storing twice, if that falls
out cleanly); `to_json` gains `schema`, so `odm inspect` shows it —
that is also how the agent learns an input's element shape.
Cascade merge/lints stay as they are, operating on the top level
(conflicting-*schema* declarations can tighten the type lint later if
it ever bites). `declared_entries` (the failed-build subset) carries
the schema too.

## 3. Panel recursion (odm-viewer-core `inputs.rs`, `tab.rs`)

- **Path addressing.** `Path` = `Vec<Seg>` with `Seg::Key(String) |
  Seg::Index(usize)`, rooted at an input name. Widget ids become
  `("input", section, name, path)`; `inputs::Field` (today: section,
  name, component index — vectors and matrices already spread one input
  over several fields) grows a `Path` in place of the index, and
  `Tab::edit` follows it. Leaf shown-value = navigate the
  top-level shown value by path; an absent optional property shows its
  nested default, else a type-blank (0, "", false, [], {}, null).
- **Events stay whole-value.** `Event::Set(section, name, value)` is
  unchanged: the panel splices a leaf edit at its path into a clone of
  the shown top-level value (creating intermediate containers for
  absent optional paths) and emits the whole value. Add/remove do the
  same. `apply`/`set_or_clear`/pruning/persistence untouched.
- **Rendering is one recursive function** over (schema, shown value,
  path):
  - Leaves (number/integer/string/boolean/enum/ext types/untyped)
    render exactly today's controls — check box, radios, number field
    with slider/adjusters, vector rows, matrix grid, text field — just
    addressed by path and indented by depth.
  - `object` with `properties`: a group header row (the input or
    property name), then one recursive row per property, indented.
  - `array` with `items`: a group with one recursive sub-entry per
    element (labelled by index), plus the Add button; per-element ×
    removes. Reorder is deferred (see Follow-ups).
  - Anything unrenderable at any depth (`object` without `properties`,
    untyped subtrees) falls back to today's JSON text field *for that
    subtree* — the universal escape hatch.
  - Reset stays one button per top-level input (whole-value clear).
- Visual design is settled at implementation time with the gallery
  open: indentation vs the 45% label column, whether groups collapse
  (`theme::collapsing` exists), keeping the era property-sheet look.
  Note the interaction with issues/panel-clips-long-input-values.md —
  indentation makes it worse; consider fixing together.
- The headless-egui unit tests in `inputs.rs` grow cases for: editing
  a nested leaf (splice correctness), nested edit-buffer isolation,
  add/remove element, JSON-fallback subtree, reset clearing the whole
  value, nested-default display.

## 4. Add / remove ("new object")

- New element value = `items.default` if declared, else synthesized
  from the items schema: nested defaults where declared, type-blanks
  elsewhere, required properties filled recursively.
- Adding pins the whole array (a grown array ≠ the default, so
  `set_or_clear` does the right thing on its own); removing the last
  added element back to the default clears the pin — for free, since
  comparison is whole-value.

## 5. Tagged unions (`variants`)

The scene case wants elements whose shape depends on a discriminant: a
box has `size`, a sphere has `radius`. Raw JSON Schema `oneOf` is a
poor fit (nothing marks the discriminant, and validation errors are a
cross-product). Instead an ODM key, in the spirit of `cascade`:

```js
shape: { variants: {
  box:    { properties: { size:   { type: 'vector3', default: [10, 10, 10] } } },
  sphere: { properties: { radius: { type: 'number', default: 5 } } },
} }
```

- **Wire form is internally tagged**: `{ kind: 'box', size: [...] }`.
  The tag property is `kind` by default; an optional `tag: '<name>'`
  key renames it. Variant bodies are object schemas (`properties`/
  `required`), since an internally tagged value is necessarily an
  object.
- **Desugar** to `anyOf` of object schemas (tag `const` + that
  variant's properties) for the `jsonschema` validator, but produce
  our own error message naming the tag when the tag itself is
  bad/missing. Normalize / hydrate / default-fill select the branch by
  tag and recurse — same walks as §1.
- **UI**: the tag renders as the enum control (radios); the active
  variant's property rows render below it, indented. Switching
  variants replaces the subtree with the new variant's template
  (nested defaults + synthesis) — no heuristic preservation of
  same-named fields; the documented pattern is to keep shared fields
  (like `position`) *outside* the union, on the enclosing object.
- Union `default` must carry a tag; a union without one synthesizes
  the first declared variant.

## 6. String-keyed maps (`additionalProperties`)

- **Profile**: `additionalProperties: <schema>` on `type: 'object'`,
  mutually exclusive with `properties`/`required` — a map, not a
  record. Allowed at any depth; the value schema is the full recursive
  grammar.
- **Identity is unordered**: canonical JSON hashing already sorts
  keys; the panel displays entries key-sorted.
- **UI**: the array control with a key column — each entry is an
  editable key text field + × + the recursive value control. A rename
  splices remove+insert (empty or duplicate keys discard like any
  invalid edit); Add inserts a synthesized value under a fresh unused
  key (`new`, `new-2`, …) with the key field focused for immediate
  rename.

## examples/input-gallery: demonstrate and test as built

The gallery is the fixture for every stage — extend it *with each
stage*, not at the end, and keep
`examples::input_gallery_covers_every_control` asserting the new
report shapes as they land:

- §1: a nested exhibit — e.g. give `hole` a nested-default property
  and a `vector3` inside an object — proving deep normalize/hydrate
  builds.
- §2: coverage test asserts `schema` is carried (items/properties
  visible for `bars`/`hole`).
- §3: the panel renders the existing `bars` (array of numbers) and
  `hole` (object) as structured controls; screenshot via the
  gui-testing skill.
- §4: the headline exhibit — an `objects` array input (object elements
  with `position: vector3` + an enum), built via one
  `ctx.invoke('parts/marker.js', obj)` per element so `"stats": true`
  shows memo hits when one object moves. Include at least one
  doubly-nested input (array inside object, or object array element
  containing a vector) so arbitrary depth is demonstrated, not just
  claimed.
- §5: upgrade the `objects` elements to `{ position, shape:
  variants{box, sphere} }` — the enum row becomes a real union.
- §6: a map exhibit (e.g. named anchors: map of `vector3`, one marker
  each), covered by the report test.

## Docs

`docs/api/inputs.md`: nested schema grammar (ext types at depth,
nested `default` semantics — applied for object properties, template
for array items — `variants`/`tag`, `additionalProperties`), the
per-element-invoke memo pattern, the shared-fields-outside-the-union
pattern, and a note that the panel renders structure while the CLI
sets whole values. Doctests for the new examples.

## Follow-ups (named, out of scope)

- **Selection ↔ panel linking** (the honest remnant of option A): an
  authored provenance tag mapping scene nodes → input paths would let
  a viewport click scroll/highlight the owning group, and eventually
  gizmos write `objects[i].position`. Needs an IR field +
  FORMAT_VERSION bump + framework API; wait for demonstrated need.
- **Doohickey-typed inputs**: `items` referencing another doohickey's
  declared plain inputs would de-duplicate the scene/part schema and
  make a real scene composer ("new object" = pick a doohickey).
- Element reorder buttons; per-path reset. (Multi-field vector leaves
  and sliders landed at the top level 2026-08-17; the recursion has to
  reuse them, not reinvent them.)
