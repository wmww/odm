# CLI

Run `odm` anywhere inside the project (it finds the nearest `odm.toml`).
A command takes **one JSON object — the whole request** — or nothing
for the defaults; single-quote it. Each prints one JSON object; exit
code 0 = ok.

```
odm status                    # files, units, view slots and their build state, selection
odm inspect ['{…}']           # measure the scene / one node
odm render  ['{…}']           # PNG → prints path
odm raycast '{…}'             # nearest surface hit along rays
odm clearance '{…}'           # signed distance between node pairs (gap/penetration)
odm feedback '{…}'            # report an ODM bug or missing feature (human-reviewed)
odm docs [<topic>]            # the full reference (bare: topic list)
odm docs search <pattern>     # grep the reference
```

`odm docs cli` has every request field of every command (exact camera
placement, view slots, index paths, `--project`).

Every scene command targets a **view**: a part (`root.js` unless
`"path"` names another) built with its declared input defaults.
`"inputs"` sets any input (`{"inputs": {"t": 1.5}}`; an unknown name
is an error listing the settable ones), `"preset"` applies one from the
target's meta, and `"view": true` starts from the user's active tab
(its path and inputs).

## Inspecting

`odm inspect` measures exactly; a render only shows shape.

```
odm inspect                                         # whole scene, summary
odm inspect '{"node": "seat"}'                      # one node, in full
odm inspect '{"node": "seat", "recursive": true}'   # …and its subtree
odm inspect '{"fields": ["name", "bounds"]}'        # just these columns
odm inspect '{"fields": ["description", "inputs", "presets"]}'  # what can I set here
```

Name your solids (`s.name('seat')`) — that is how nodes are addressed.
Every entry has `id`, `name`, and world `bounds` and `tris` for its
whole subtree, so the root's bounds are the model's extent. Identical
siblings collapse into one entry with `repeat: N`. `"full": true` adds
`volume`, `area` and placement, and expands repeats. The interface
fields report even when the build fails.

An assembly can look right from one angle and still float or collide. After
assembly edits, `odm clearance '{"pairs": [["seat", "frame"]]}'` gives
a signed `distance` per pair (positive = exact gap with the closest
points; negative = penetration with a translation that clears it) and
names the colliding leaves. Threshold `|distance|` for contact; don't
nudge geometry to fix a near-zero sign.

## Rendering

```
odm render                                                   # framed overview
odm render '{"look": "top"}'                                 # orthographic plan view
odm render '{"look": "left"}'                                # side elevation
odm render '{"wireframe": true, "width": 1600}'              # edges only
odm render '{"opacity": 0.3}'                                # x-ray: see inside
odm render '{"inputs": {"t": 2.5}, "out": "/tmp/frame.png"}' # one animation moment
odm render '{"path": "parts/wheel.js", "inputs": {"radius": 12}}'  # one part alone
```

`look` takes `top`/`bottom`/`front`/`back`/`left`/`right` (orthographic
drafting views) or a vector to gaze along. The camera always frames
the model, and every response echoes the resolved `camera`: nudge its
numbers and paste them back at the top level for exact placement.
Don't read dimensions off pixels — `inspect` is exact.
