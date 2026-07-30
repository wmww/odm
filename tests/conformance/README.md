# Conformance suite

Per-API-version test doohickeys asserting what the JS API *means*. One
directory per version channel. `unstable/` is mutable and grows with every
feature and every bug found — it is the seed of the future v1 suite. From v1
on, a stamped version's suite follows the contract in `docs/versioning.md`:
add tests freely, port to new runner infra, weaken or remove assertions
never; asserting a bugfix in a stamped version is a deliberate per-case
decision (the default is bug-compatibility with the original behavior).

The runner is `crates/odm-engine/src/conformance.rs`
(`cargo test -p odm-engine conformance`). It treats each entry here as one
test:

- `name.js` — a single-file project (the file becomes its `main.js`).
- `name/` — a whole project directory (`main.js`, parts, optional
  `odm.json`), for invoke/params/animation tests.

Every test file carries its `//! odm <version>` pragma and exports its
assertions next to its geometry:

```js
//! odm unstable
export default function build(ctx) { return odm.box(10); }

export const checks = [
  { volume: [1000, 1e-6], area: [600, 1e-6] },
  { bounds: { min: [-5, -5, -5], max: [5, 5, 5], eps: 1e-6 } },
  { raycast: { origin: [0, 0, 100], dir: [0, 0, -1], distance: [95, 1e-6], normal: [0, 0, 1] } },
  { t: 2, volume: [3000, 1e-6] },        // any check can set its context
  { error: 'must be Solids' },           // build must fail, message contains
  { console: ['made 4 wheels'] },        // each substring appears in the logs
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
- `error: 'substring'` — the build at this check's `t` must fail and the
  message must contain the substring. The module itself must still
  evaluate (checks live in it), so this is for build()-time errors.
- `console: ['substring', …]` — each must appear in that build's logs.
- `t: seconds` — context for this check (default 0).

Values pinned by a check must be *derivable* (analytic, or exact CSG
arithmetic), not pasted from whatever the engine printed — a suite seeded
with the current output would only ever prove the engine equals itself.
Discretization-dependent values (default segment counts are API surface)
get a loose eps sanity-checked against the analytic value.

Golden renders with image-diff tolerance are planned but deferred until
pinned-driver CI exists (renders are only comparable per GPU/driver).
