# One scene query command (inspect subsumes tree)

## Why
Live test: `odm tree` on a 139-instance swing set printed 191 KB (matrices as
16 lines each, 30 identical chain links each fully expanded), while `inspect`
only takes index paths — so "check the seat" meant dumping the tree to find
`1/0/1`, or raycasting to fish for an id. `tree` and `inspect` are really one
query at two hardcoded corners of a (scope × detail) grid, with mismatched
field names (`matrix` vs `world_matrix`, `world_bounds` vs `bounds_world`).

## Design
One command, `inspect`; `tree` goes away (`docs` gets a pointer).
Two orthogonal knobs:

- **scope**: node (by name or index path, default root) + recursion depth.
- **detail**: summary (default) | `--full` | `--fields name,bounds,volume`.

Defaults keyed on whether a node was named — both archetypal questions stay
zero-flag:

```
odm inspect                    # whole scene, recursive, summary
odm inspect seat               # that node, all detail, children as counts
odm inspect seat --recursive   # subtree with detail
odm inspect --full             # exhaustive dump (today's tree, unified shape)
```

### Summary fields
`name`, `id`, aggregate world bounds (groups too — today only mesh nodes have
bounds, so "how tall is the whole model / does it sit on z=0" is unanswerable),
`tris`, child count. Runs of siblings sharing a mesh hash collapse to one
entry with `repeat: N` — repeated parts are the intended invoke pattern, the
tree should show them that way. Color only when explicitly set (sRGB — see
`colors-srgb.md`).

### Full/fields detail
- Transform decomposed as position/rotation/scale (raw matrix via
  `--fields matrix`; note decomposition is lossy under shear).
- Kernel measurements (`volume`, `surface_area`, `verts`) computed on demand —
  fine when asked for, wrong as a recursive default.
- Drop from all output: mesh hashes (internal; their one payoff — spotting
  repeats — is delivered by `repeat: N`), `bounds_local` (world bounds is the
  measurement tool; local is a JS-side concern).

### One node schema
Same recursive node shape and field names at every level and everywhere a node
is referenced (`raycast`, `selection` included).

### Name addressing
Names already exist on nodes and appear in output; there is just no lookup
(`find_node_world` is index-path only). `inspect seat` resolves by name;
ambiguous name → error listing the matches with ids (cheap, agent-friendly).
Index paths remain as the tiebreaker/fallback.

## Notes
- Serialization lives in `odm-engine/src/scene.rs` (`tree_json`,
  `find_node_world`) and `commands.rs` (tree/inspect handlers).
- Trade-off accepted: an agent wanting all transforms at once now needs
  `--full`/`--fields` where today's tree had them by default.
- Prompt: teach `inspect` in the core loop; delete the tree section.
