# Assembly checks: `query clearance`

Builds on `cli-json-args.md` (a `query` kind; no flag spelling ever exists).

## Why
The one real modelling bug in the live test: a seat centred 15 cm forward of
its chains. It rendered fine from the default isometric angle (offset was
along the view direction) and was only caught by the user in plan view.
"Is A attached to B / do these collide" is the most common assembly
correctness question, and there's no direct way to ask it — raycast only
confirms after you've guessed where the part should be, which is backwards.

## Design
A plural-native query kind: `pairs` of nodes (names / index paths, as
`inspect` addresses them), all measured against the request's one view spec,
answered in order:

    odm query clearance '{"pairs": [["seat", "chainL"], ["seat", "chainR"]], "inputs": {"t": 1.5}}'

One command checks a whole assembly's contact pairs after an edit — the
scriptable self-check — and works at animation extremes for collision
checks.

**JS twin (required, per the parity rule)**: `a.clearance(b)` on solids,
same result shape — doohickeys can assert their own fit.

## Response shape — honest per tier
No signed distance. Tier 1's AABB gap is only a lower bound (gap 0 ≠ contact
— think interlocking L-shapes), tier 2 is a boolean, and even exact min
distance is 0 for overlap, never negative (penetration *depth* is a
different, harder query than min distance).

- Tiers 1+2, per pair: `{"overlap": bool, "gap_lower_bound": n}`.
- Tier 3 upgrades to
  `{"overlap": bool, "distance": n, "closest": [[x,y,z], [x,y,z]]}` —
  `distance` exact and ≥ 0; the closest-point pair tells the agent *where*
  the gap is.

Tiered implementation — each tier is useful alone:

1. **AABB distance** between subtree world bounds: cheap lower bound on the
   gap; already catches "seat is 15 cm from where the chains end".
2. **Overlap test** via Manifold boolean intersection (exists): nonzero
   volume = interpenetrating. Answers "touching/overlapping: yes/no".
3. **Exact min distance**: manifold-csg 0.3.3 has no distance query, so this
   is custom — triangle BVH per mesh in `odm-kernel`, BVH-vs-BVH traversal
   with triangle-triangle distance. Decide if tier 1+2 cover enough in
   practice before building it.

## Maybe (separate, weaker complement)
A build-report lint flagging subtrees whose bounds touch nothing else
("floating part") would have caught the seat with no arguments at all — but
it's heuristic (grounded-by-gravity parts, intentional gaps); design
carefully or drop.
