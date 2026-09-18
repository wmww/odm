# Raycast hits carry the nearest named ancestor

Resolves `issues/instance-name-does-not-reach-raycast.md`.

## The problem

`raycast` (CLI `hits[].name`, viewer picks, web-export picks) reports
the name of the node that *owns the mesh*. A name set on anything above
it — an `Instance` from `ctx.invoke`, a `Group` — is invisible to the
hit. For an invoked part the wrapper is the only handle the caller has,
so four `ctx.invoke('wheel.js').name(...)` copies raycast identically.

`clearance` already solved this: its leaf labels are "own name, else
nearest named ancestor within the queried subtree, else index path"
(`collect_meshes`, crates/odm-engine/src/scene.rs; policy recorded in
notes/agent-surface.md). Raycast is the one query that disagrees.

## Decision

A hit's `name` is the hit node's own name, else its nearest named
ancestor's, else `null`. `id` stays the mesh node's index path.

Rejected:
- **Name path** (`"chain/link"` or an array): changes the result shape
  and the conformance `name` check for little gain — `inspect` on the
  hit's `id` already gives the path.
- **Push the wrapper's name into the invoked subtree**: the subtree is a
  content-addressed ref shared by every copy; the wrapper is the only
  place a per-copy name can live.
- **Keep groups unnamed, fix only Instances**: two rules where one will
  do; the `names.js` pin that a group name stops at the group describes
  an accident, not a wanted behaviour.

Nearest wins over farthest so an author can still label a sub-part
inside a named assembly and hit *that*.

## Where the change lands

One place: `walk` in crates/odm-render/src/flatten.rs already threads
inherited color and opacity down the tree. Thread the name the same way
and store the resolved value in `Instance.name`. Every consumer reads
that field and needs no change:

| consumer | file |
| --- | --- |
| CLI raycast / conformance `raycast` checks | odm-engine `scene::raycast` |
| viewer click pick and wire pick, selection reported in `status` | odm-viewer-core viewer.rs `pick_*`, odm-engine `selection_json` |
| web export pick | odm-web host.rs |

Note the flattener is the "single scene flattener" by design (its doc
comment) — this is exactly why the fix belongs there and not in
`scene::raycast`.

`collect_meshes` stays separate: clearance labels are scoped to the
*queried subtree* (an ancestor above the queried node does not count),
so it cannot read the whole-scene flattened name. Point its comment at
the flattener so the two rules are visibly the same.

## Steps

1. **Flattener.** Add `inherited_name: Option<&str>` to `walk`;
   `let name = node.name.as_deref().or(inherited_name)`; push
   `name.map(str::to_string)` into the instance; pass `name` to
   children. Update `Instance.name`'s doc in odm-render lib.rs: "own
   name, else nearest named ancestor". Add a test in
   crates/odm-render/tests/flatten.rs: named group over an unnamed mesh
   → group's name; named mesh inside a named group → the mesh's own
   name; unnamed everywhere → `None`.

2. **Docs.** Say the rule wherever a hit's `name` is described:
   - `requests.rs` raycast spec doc string (regenerates the block in
     docs/cli.md between the GENERATED markers — rebuild, don't hand
     edit that block).
   - docs/cli.md hand-written `### raycast` paragraph (~line 326) and
     the geometry-queries intro line "hits carry `id`/`name`" (~316).
   - docs/api/queries.md, `s.raycast` section: one line that the CLI
     twin adds `id`/`name`, with the rule.
   - docs/api/transforms.md `s.name(n)` comment and
     docs/api/composition.md `## Instance`: naming an Instance or Group
     is how a raycast hit inside it is attributed.

3. **Conformance pins.**
   - tests/conformance/unstable/names.js: the `crate` ray now expects
     `name: 'crate'`; rewrite the comment. Add a named solid inside a
     named group and pin that the solid's own name wins.
   - tests/conformance/unstable/invoke/root.js: the copy's top-face ray
     gains `name: 'copy'`; drop the apologetic comment. Add a ray onto
     the `plate` for `name: 'plate'` so two copies of one ref are told
     apart — the motivating case.
   - Check the JS-twin docs' doctests (`odm docs` examples) still pass;
     none should mention hit names, but confirm.

4. **Notes and issue.** In notes/agent-surface.md's clearance bullet,
   generalize: "Labels = ... — raycast `name` uses the same rule
   (whole-scene ancestors; clearance's are scoped to the queried
   subtree)". Delete the issue file.

5. **Verify.** `cargo test --workspace` (flattener unit test, scene.rs
   raycast tests, conformance suite). Then a live check with
   `odm run --headless` on the invoke conformance project: raycast at
   `[24, 0, 50]` returns `"name": "copy"`.

## Watch for

- `scene.rs` has its own raycast unit tests building `Instance`s by
  hand; they are unaffected but should keep constructing `name` as the
  flattener now would.
- The viewer's selection shows `(id, name)` in the tree and to the
  agent through `status`. With inheritance a pick on an unnamed leaf
  under `wheel-fl` now reports `wheel-fl` with the leaf's id — this is
  the intended reading ("you selected the wheel") and matches what the
  tree reveals by id.
- `issues/numeric-names-collide-with-index-paths.md` is adjacent
  (names vs index paths in addressing) but independent; do not fold it
  in.
