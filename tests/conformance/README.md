# Conformance suite

Per-API-version test parts asserting what the JS API *means*. One
directory per version channel. `unstable/` is mutable and grows with every
feature and every bug found — it is the seed of the future API 1 suite. From API 1
on, a stamped version's suite follows the contract in `docs/versioning.md`:
add tests freely, port to new runner infra, weaken or remove assertions
never; asserting a bugfix in a stamped version is a deliberate per-case
decision (the default is bug-compatibility with the original behavior).

The runner is `crates/odm-engine/src/conformance.rs`
(`cargo test -p odm-engine conformance`). It treats each entry here as one
test:

- `name.js` — a single-file project (the file becomes its `root.js`).
- `name/` — a whole project directory (`root.js` plus parts), for
  invoke/inputs tests.

Every test file carries its `//! ODM API <version>` pragma and exports its
assertions next to its geometry:

```js
//! ODM API unstable
export default function build(ctx) { return odm.box(10); }

export const checks = [
  { volume: [1000, 1e-6], area: [600, 1e-6] },
  { bounds: { min: [-5, -5, -5], max: [5, 5, 5], eps: 1e-6 } },
  { raycast: { origin: [0, 0, 100], dir: [0, 0, -1], distance: [95, 1e-6], normal: [0, 0, 1] } },
  { t: 2, volume: [3000, 1e-6] },        // shorthand for set: { t: 2 }
  { set: { width: 3 }, volume: [36, 1e-6] },  // view-level inputs for this check
  { error: 'must be Solids' },           // build must fail, message contains
  { console: ['made 4 wheels'] },        // each substring appears in the logs
  { node: 'seat', color: '#ff0000', opacity: null, volume: [8, 1e-9] },
  { flat: [['#ff0000', 1], [null, 0.25]] },  // effective colors, any order
  { meshes: 1 },                         // distinct mesh hashes in the scene
];
```

Semantics (never byte-exact — Manifold upgrades legitimately change
triangulation):

- `volume: [expected, eps]` — Σ over scene instances of mesh volume ×
  |det| of the instance's world transform. Overlapping instances count
  twice; union first if that matters.
- `area: [expected, eps]` — Σ of untransformed per-mesh surface area.
  Instance transforms are IGNORED (area doesn't compose under non-uniform
  scale); assert area on untransformed geometry only.
- `bounds: {min, max, eps}` — world AABB of the scene.
- `raycast: {origin, dir, ...}` — nearest world-space hit;
  `distance: [d, eps]`, `normal: [x,y,z]` (compared within `eps`, default
  1e-6), `name: 'node name'`, or `miss: true`.
- `error: 'substring'` — the build with this check's inputs must fail and
  the message must contain the substring. The module itself must still
  evaluate (checks live in it), so this is for build()-time and
  input-boundary errors.
- `console: ['substring', …]` — each must appear in that build's logs.
- `set: { name: value }` — view-level inputs for this check; split into
  args/cascade against root.js's meta, like a CLI request's `inputs`.
- `t: seconds` — shorthand for `set: { t: … }` (default 0).
- `node: 'name' | '1/0/2'` — address one node (`scene::locate`, the way
  `odm inspect` addresses them). `volume` and `bounds` in the same check
  then measure **that node's subtree** (world-transformed, via
  `Inspector`) instead of the whole scene, and `color`/`opacity` below
  become available. `area` is scene-wide only and may not be combined
  with it.
- `color: '#rgb' | '#rrggbb' | '#rrggbbaa' | [r,g,b(,a)] | null` — the
  color *authored* on the addressed node, not the inherited one; `null`
  asserts that nothing was set there. Needs `node`.
- `opacity: x | null` — likewise for the authored opacity (chained
  `.opacity()` calls collapse into one value). Needs `node`.
- `flat: [[color, alpha], …]` — the multiset of **effective** per-instance
  colors after inheritance and opacity, straight out of
  `odm_render::flatten_node` — what the renderer will actually draw.
  Compared order-insensitively (rounded to 1e-6), so the list is one entry
  per drawn instance in any order. `color` takes the same literals as
  above, with `null` meaning "no color anywhere up the tree" (the
  flattener's neutral default); `alpha` is the color's own alpha times the
  product of the ancestors' opacities.
- `meshes: n` — the number of distinct mesh hashes the scene reaches.
  Pins "shared geometry is interned once": two placements of one solid are
  `meshes: 1`.

## Coverage

`every_api_name_is_exercised` (in the runner) reads the *live* API
surface — `Object.keys(odm)` plus every non-underscore method up the
prototype chains of `Solid`/`Group`/`Instance` — and asserts each name
appears somewhere in this directory as `.name(`, `odm.name(` or
`new odm.Name(`. Reading it off the surface rather than a list is the
point: adding a function to `installGlobals` fails the gate until it has
a test. `Solid`, `Group`, `Instance` and `children` are allowlisted —
they are exercised by construction and `instanceof`, never called by
name.

Values pinned by a check must be *derivable* (analytic, or exact CSG
arithmetic), not pasted from whatever the engine printed — a suite seeded
with the current output would only ever prove the engine equals itself.
Discretization-dependent values (default segment counts are API surface)
get a loose eps sanity-checked against the analytic value.

Golden renders with image-diff tolerance are planned but deferred until
pinned-driver CI exists (renders are only comparable per GPU/driver).
