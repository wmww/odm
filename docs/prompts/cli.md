# CLI

The `odm` CLI talks to the running engine; run it from anywhere inside
the project. Every scene command **syncs first** (rescans files,
rebuilds what changed, then answers) and prints one JSON object; exit
code 0 = ok. A command takes **one JSON object — the whole request** —
or nothing for the defaults. Single-quote it.

```
odm status                    # files, units, view slots and their build state, selection
odm inspect ['{…}']           # measure the scene / one node
odm render  ['{…}']           # PNG → prints path
odm raycast '{…}'             # nearest surface hit along rays
odm clearance '{…}'           # per node pair: signed distance (gap/penetration)
odm feedback '{…}'            # report an ODM bug or missing feature (human-reviewed)
odm docs [<topic>]            # the full reference (bare: topic list)
odm docs search <pattern>     # grep the reference
```

`odm docs cli` has every request field of every command, exact camera
placement, viewer view slots, index paths and `--project`.

Every scene command targets a **view**: a doohickey (`root.js` unless
`"path"` names another file) built with its input defaults. `"inputs"`
sets any input — `odm render '{"inputs": {"t": 1.5}}'`; an unknown
name is an error listing the settable ones. `"preset"` applies one of
the target's presets first. `"view": true` targets what the user is
looking at (their tab, their input values).

`odm inspect '{"fields": ["description", "inputs", "presets"]}'` answers
"what can I set here": every settable input with its value, type/range,
default and declaration site, plus the presets — even when the build
fails.

## Inspecting

`odm inspect` measures exactly; a render only shows shape.

```
odm inspect                                          # whole scene, recursive summary
odm inspect '{"node": "seat"}'                       # one part in full, children as a count
odm inspect '{"node": "seat", "recursive": true}'    # …and its subtree
odm inspect '{"fields": ["name", "bounds"]}'         # just these columns
```

A node is addressed by the `name` you gave it (`s.name('seat')`) — name
your parts. Every entry has `id`, `name`, world `bounds` and `tris` for
its **whole subtree**, so the root's bounds are the model's extent. Runs
of identical siblings collapse into one entry with `repeat: N`.
`"full": true` adds `verts`, `volume`, `area` and the node's own
placement; `"fields"` picks exactly the columns you want.

A misplaced part can look right from one angle. After assembly edits,
`odm clearance '{"pairs": [["seat", "frame"]]}'` checks that parts
meet: per pair a signed `distance` (positive = exact gap with the
closest points; negative = penetration with a translation that clears
it) and `between`/`overlapping` naming the colliding leaves. A
near-zero sign is float noise: threshold `|distance|` for contact
instead of nudging geometry.

## Rendering

```
odm render                                                   # framed overview
odm render '{"look": "top"}'                                 # plan view (also bottom/front/back/left/right)
odm render '{"look": [-1, 0, -0.4]}'                         # gaze along a vector
odm render '{"wireframe": true, "width": 1600}'              # edges only
odm render '{"opacity": 0.3}'                                # x-ray
odm render '{"inputs": {"t": 2.5}, "out": "/tmp/frame.png"}' # one animation moment
odm render '{"path": "parts/wheel.js", "inputs": {"radius": 12}}'  # one part alone
```

The camera always frames the model (`look` keywords are orthographic
drafting views; a vector is perspective). `width`/`height` default
1024×768, `out` defaults under `.odm/renders/`, `no_grid` hides the
grid. Every response echoes the resolved `camera`; its contents are
render fields, so nudge the numbers and paste them back at the top
level for exact placement. `focus`, `zoom`, `eye`, contact sheets
(`frames`): `odm docs cli`. Don't read dimensions off pixels —
`inspect` is exact.

## Talking with the user

ODM runs you: what the user types into the viewer's Agent panel arrives
as a message here, and your replies appear there as you write them.
There is nothing to poll or send — just answer, briefly and in plain
text (the panel shows text as typed, and the rebuilt scene speaks for
itself). The viewer logs each command you run and each file you change
next to what you say, derives your working status from your turn, and
lets the user stop a turn.

- Each user message carries an `odm://user-state` attachment: what they
  were looking at as they sent it — tab `slot` and `path`, its
  `inputs`, their `selection` (clicked parts as `{id, name}`, in pick
  order) and the `camera`. When they say "this part", `selection` has
  it. Paste `camera`'s contents into `odm render` to see exactly what
  they saw. `odm status` shows the current selection on demand.
- A message starting `[odm engine]` is from the engine, not the user: a
  build broke (or a file's background check failed) while you were
  idle, or your turn ended with something still failing. The errors are
  attached as `odm://diagnostics`. Fix what it names; nobody is waiting
  on a reply.
- `odm` commands and edits inside the project run without asking;
  anything else may ask the user for permission first.

## Reporting ODM problems

Hit a bug in ODM itself, or a wall `odm docs` has no answer for? File
it and carry on:

```
odm feedback '{"title": "…", "body": "…", "harness": "…", "model": "…"}'
```

Paste a minimal reproduction and the exact request and response into
`body` (there are no attachments). The report is written into the
project for the user to review and send; there is no reply to wait
for. Bugs in the project you are building are yours to fix, not to
report.
