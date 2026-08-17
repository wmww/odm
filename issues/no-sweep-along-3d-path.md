# No way to sweep a profile along a 3D path

From agent feedback (real session, 2026-08): the THREE subset has `Shape`
and `Path` (2D only) — no `Curve3`, `CatmullRomCurve3`, `LineCurve3`,
`CurvePath`, or `TubeGeometry` — and `odm.extrude` only goes straight up +Z.
So there is no primitive for "rope", "cable", "hose", "wire", "bent tube",
which are everywhere in real assemblies. The agent's options were a
cylinder-plus-sphere per segment (hundreds of nodes for one lacing run) or
hand-rolling a parallel-transport tube into a `BufferGeometry` via
`fromThreeGeometry` — ~60 lines of frame math every project needing a curved
tube will rewrite.

Quote: "An `odm.sweep(profile, points, {caps})` would be the single
highest-value addition I hit today."

Doc papercut until then: `docs three` says "2D: `Shape`, `Path` (with the
full curve API...)" which reads as though 3D curves might be nearby, and the
AGENTS.md short prompt says the subset includes "curves" flatly. Say "2D
curves only, no 3D curve classes" outright.
