# Signed distance for `clearance`

Upgrade `clearance` (both surfaces) from `{overlap, gap_lower_bound}` to a
signed `distance`: positive = exact minimum gap, negative = penetration,
zero = exact contact. Closes tier 1+2's blind spot (boxes touch, solids
don't → today reads an uninformative 0) and answers "how badly do they
collide, and which way out".

## Correction to the old premise

plans/clearance.md said "manifold-csg 0.3.3 has no distance query". Wrong:
`Manifold::min_gap(other, search_length) -> f64` exists in our pinned
0.3.3 (Manifold's collider underneath). What it does NOT give: closest
points, and anything for the overlapping case. So the positive side is
mostly free; the negative side is the real work.

## Semantics — decide first, it's the hard part

**Positive (disjoint)**: exact min distance between the solids. Easy to
define, exact to compute. `closest: [[x,y,z],[x,y,z]]` tells the agent
*where* the gap is.

**Negative (overlap)**: the meaningful magnitude is penetration depth =
minimum translation distance (MTD): the smallest move that separates.
Exact MTD for non-convex meshes is intractable (Minkowski difference of
non-convex polyhedra; no kernel help). Honest-per-tier proposal:

- Report `distance = -s` where `s` is the magnitude of the **best found
  separating translation** — an upper bound on true depth, but a
  *guarantee*: applying it separates the parts. Actionable ("chainL is
  ~2.1 into the seat; +z by 2.1 clears it"), which is what the agent
  needs; minimality is not.
- Report the vector too: `separate: [x,y,z]` (move the pair's *second*
  node by this to clear the first). This replaces `closest` on the
  negative side.
- Sign stays exact: any negative value proves real overlap (tier 2's
  test decides the sign, not the search).
- Document the asymmetry: positive exact, negative an upper bound.

Rejected: min-width-of-intersection (wildly underestimates for crossing
parts — two crossing plates read plate-thickness while separating needs a
half-plate slide); boundary-to-boundary distance with a flipped sign
(reads 0 for every generic partial overlap — the main use case).

Containment (A entirely inside B) needs no special case under this
definition: the separating translation pushes A out through the thinnest
wall, and the search handles it like any overlap.

## Result shape

Per pair / per JS call:

- disjoint: `{"distance": 2.5, "closest": [[…], […]]}`
- overlapping: `{"distance": -1.2, "separate": [0, 0, 1.2]}`

`overlap` and `gap_lower_bound` are dropped — the sign carries `overlap`
exactly, and exact distance supersedes the bound. Unstable channel, no
stamped version yet, so the break is allowed; update the conformance
suite (unstable may be reshaped) and every doc that names the old keys.
JS parity as with raycast: same keys, points hydrate to `Vector3`s.

No new request fields: exact is the only mode (no `"exact": true` knob).
BVH work is cached per mesh hash and pruned by AABB bounds; the negative
search is iteration-bounded. If profiling on real assemblies says
otherwise, revisit — don't pre-add the knob.

## Implementation

Phase 0 — **spike `min_gap`**: confirm on 0.3.3 that overlap → 0,
empty → what, cost on ~100k-tri meshes, and that `search_length` caps
the answer (seed it from the AABB gap + slack). If it holds, the
positive-side *number* needs no new geometry code at all.

Phase 1 — **triangle BVH in odm-kernel** (needed regardless, for
everything min_gap doesn't do):

- Per-mesh BVH cached by content hash next to the Manifold cache; same
  `clear_cache`/`prune_cache` lifecycle.
- Queries take (Hash, Transform) operands like everything else: prune
  with transformed node AABBs (conservative under affine), transform
  triangle vertices at the leaves — no baking, no per-transform cache.
- `closest_points(a, b) -> (dist, [p, q])`: BVH-vs-BVH branch-and-bound
  with triangle-triangle distance; multi-operand sides via one priority
  queue over operand pairs seeded with AABB gaps (subtree-vs-subtree
  falls out).
- `surfaces_intersect(a, b) -> bool`: tri-tri intersection test, plus
  one containment raycast (parity) when surfaces are disjoint. This is
  the fast overlap predicate — replaces the CSG intersection in today's
  tier 2 and is what makes the negative search affordable.

Phase 2 — **negative side**: separating-translation search.

- Candidate directions: normals harvested from the intersecting triangle
  pairs found by the overlap predicate, the centroid-difference vector,
  and the coordinate axes.
- Per direction: bisect the translation magnitude on the fast overlap
  predicate (≈20 tests) for the smallest separating `s`; keep the best
  direction, then hill-climb around it a few steps.
- Bound total iterations; the result is valid (a separating translation)
  whenever the loop exits with one, and the reported magnitude only ever
  overestimates depth.

Phase 3 — **surface swap**: kernel `Clearance` struct, `cmd_clearance`,
`op_clearance`, `Solid.clearance`, docs (cli.md manual + generated
section stays as-is field-wise, `docs/api/queries.md`, both prompt
files), conformance (`clearance.js` rewritten for the new shape),
runtime + kernel + scene tests, notes/agent-surface.md's "no signed
distance" standing decision replaced by the new contract.

Optional hardening (separate, later): a rigorous *lower* bound on depth
via the inradius of the Manifold intersection (a ball of radius r inside
A∩B forbids any separating translation shorter than r), reported only if
agents demonstrably need "at least this deep" — otherwise skip.

## Testing (analytic only, per conformance rules)

- Boxes overlapping by 0.5 on an axis → distance −0.5 exactly (axis
  candidates make the upper bound tight here); identical coincident
  cubes → −(edge length); nested cubes → −(wall distance + inner
  half-extent) through the thinnest wall.
- Interlocked-but-clear L-shapes → *positive* exact distance (the case
  tier 1 could not answer).
- Sphere–sphere: distance = center distance − radii within segment eps.
- `closest` endpoints lie on the respective surfaces; `separate` applied
  via a translate must flip the sign to positive (self-verifying check).
- Transforms: rotated/scaled operands agree with baked equivalents.

## Open questions

- min_gap vs custom BVH for the positive *number*: if the spike is
  clean, use min_gap and let the BVH serve closest points + overlap
  predicate only — or use the BVH for everything and keep min_gap as a
  cross-check in tests. Decide in phase 1 by which is less code.
- Cancellation: BVH queries are ms-scale so probably exempt (like
  raycast); the phase-2 search should still check the token between
  directions if profiling says it can reach tens of ms.
