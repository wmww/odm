# Assembly checks: "do these two parts actually touch?"

## Why
The one real modelling bug in the live test: a seat centred 15 cm forward of
its chains. It rendered fine from the default isometric angle (offset was
along the view direction) and was only caught by the user in plan view.
"Is A attached to B / do these collide" is the most common assembly
correctness question, and there's no direct way to ask it — `raycast` only
confirms after you've guessed where the part should be, which is backwards.

## Design
`odm clearance <a> <b>` (nodes by name, per `scene-query.md`): minimum
distance between two subtrees; 0 or negative = contact/interpenetration.
Scriptable as a self-check after each edit, and usable at animation extremes
(`--set t=…`) for collision checks.

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

## Open questions
- JS-side `Solid.clearance(other)` too? Cheap once the kernel query exists;
  lets doohickeys assert their own fit. Not needed for the CLI value.
- Report the closest point pair (not just the distance) — likely worth it,
  tells the agent *where* the gap is.
