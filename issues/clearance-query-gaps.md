# clearance: uninformative bound, no offending pair, no tolerance

From agent feedback (real session, 2026-08), three gaps in `clearance`, all
hit while assembly-checking one model:

1. **`gap_lower_bound` is almost always 0 and carries no information.** For
   any pair that nests or rests on another — a pipe in a socket, a tarp on a
   beam, a cord round a pipe — the bounding boxes touch, so the bound is 0
   and only the `overlap` boolean says anything. That is the *common case*
   for assembly checking. Even a coarse sampled lower bound would beat 0.
   (`requests.rs` already documents the bbox limitation; the ask is a better
   bound, not better docs.)

2. **On two groups it says *that* something overlaps, never *what*.**
   `["roof", "beams"]` came back `overlap: true` with 3 groups × 7 beams
   underneath; the agent bisected by hand over four extra round trips.
   Returning the first (or every) offending leaf pair —
   `{"overlap": true, "between": ["lacing-north", "beam-ew-0-1"]}` — would
   turn a manual search into one call.

3. **Exact tangency is a coin flip; no tolerance knob.** Parts that
   physically rest on each other land on exact tangency, where `overlap` is
   decided by floating-point luck — identical cord-on-flange arrangements
   flipped true on one edge, false on another. The agent ended up scattering
   a hand-tuned 0.002' nudge through the model to keep contacts unambiguous,
   i.e. fudging geometry to satisfy a query. A `tolerance` field on the
   request (ignore interpenetration below X) would let models stay honest.

Positive, for contrast: unknown node names already list the scene's real
names, and that was praised — keep it.
