# inspect papercuts: full×fields exclusivity, volume not on subtree

From agent feedback (real session, 2026-08), two `inspect` papercuts:

1. `full` and `fields` are mutually exclusive, so there's no way to say
   "everything, plus keep it to these columns". Hit first try — `fields`
   narrowing `full` would be the intuitive reading.

2. `volume` lives on the Solid, not the named Instance wrapping it, so
   `inspect '{"node":"lacing-north","full":true}'` has no `volume` key — you
   have to go recursive to reach the child. Since `bounds`/`tris` are
   reported for the whole subtree on every entry, `volume`/`area` reading as
   subtree totals too would be consistent.
