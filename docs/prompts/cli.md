# CLI reference

The `odm` CLI talks to the running engine. Run it from anywhere inside
the project — it finds the nearest `odm.toml` at or above cwd. Every
command that looks at the scene **syncs first** — rescans files,
rebuilds what changed, then answers — so it always reflects your latest
edit, and prints a single JSON object. Exit code 0 = ok, nonzero =
error. (`docs` is the exception: no engine, and markdown rather than
JSON.)

One grammar: a command takes **one JSON object — the whole request** —
or nothing for the defaults. Single-quote it.

```
odm status                    # files, view slots and their build state
odm inspect ['{…}']           # measure the scene / one node
odm render  ['{…}']           # PNG → prints path
odm raycast '{…}'             # nearest surface hit along rays
odm clearance '{…}'           # per node pair: signed distance (gap/penetration)
odm feedback '{…}'            # report an ODM bug or missing feature (human-reviewed)
odm docs [<topic>]            # full reference (list topics when bare)
odm docs search <pattern>     # grep the reference, whole sections out
```

This page is the short version; `odm docs cli` is the full one — every
request field of every command, explicit camera placement, viewer view
slots, index paths, `--project`.

Hit a bug in ODM itself, or a wall `odm docs` has no answer for? File
it — `odm feedback '{"title": …, "body": …, "harness": …, "model": …}'`
— with a minimal reproduction pasted into the body. It is written into
the project for the user to review and send; there is no reply to wait
for, so file it and carry on. (Bugs in the project you are building are
yours to fix, not to report.)

Every scene query targets a **view**: a doohickey (default `root.js`,
or `"path"` names another file) built with its declared input defaults.
`"inputs"` sets any input — `odm render '{"inputs": {"t": 1.5}}'`; a
typo'd name is an error listing the settable inputs. `"preset"` applies
a preset from the target's meta first.

`odm inspect '{"fields": ["inputs", "presets"]}'` is the one answer to
"what can I set here": every settable input with its current value,
type/range, default, and where it is declared, plus the presets — on
the root entry, because the root *is* the view. A failed build still
reports them next to the error.

## Inspecting the scene

`odm inspect` is the measuring tool: it answers "what is in here" and
"how big is this" exactly, where a render only shows you shape.

```
odm inspect                             # whole scene, recursive, summary
odm inspect '{"node": "seat"}'          # that part, in full, children as a count
odm inspect '{"node": "seat", "recursive": true}'   # ...and its subtree
odm inspect '{"fields": ["name", "bounds"]}'        # narrow the columns instead
```

A node is addressed by the `name` you gave it (`s.name('seat')`) — name
your parts. Bare `inspect` is a whole-scene overview; naming a node
asks about that node, so it comes back in full detail with its children
as a count. `"depth"`/`"recursive"` set how far to expand.

Every entry has `id`, `name`, world `bounds` and `tris` **for its whole
subtree** — so the root's bounds are the model's overall extent, and a
group's are the group's. Runs of identical siblings collapse into one
entry with `repeat: N`; they differ only in placement. `"full": true`
adds `verts`, `volume`, `area` and the node's own placement, and
expands the repeats; `"fields"` picks exactly the columns you want.

A misplaced part can look right from one camera angle. After assembly
edits, `odm clearance '{"pairs": [["seat", "frame"]]}'` checks that
parts actually meet: per pair a signed `distance` (positive = exact
gap + the closest points, negative = penetration + a translation that
clears it), `between`/`overlapping` naming the colliding leaves. The
sign of a near-zero distance is float noise — threshold `|distance|`
for contact instead of nudging geometry. Details: `odm docs cli`.

## Rendering

Request fields: `width`/`height` in pixels (default 1024×768), `out`
(default under `.odm/renders/`), `wireframe: true` (edges only, in
each object's own color — surfaces are not drawn), `no_grid`,
`opacity` (0..1: x-ray, everything translucent), `look` (`"top"`,
`"bottom"`, `"front"`, `"back"`, `"left"`, `"right"` — orthographic
drafting views — or a vector to gaze along; default a framed
overview). The camera always frames the model, and every response
echoes the resolved `camera`; its contents are render fields — nudge
the numbers and paste them back at the top level (no `camera` wrapper)
for exact placement. More (`focus` on one part, `zoom`, `eye`, …):
`odm docs cli`.

```
odm render                                                   # framed overview
odm render '{"look": "top"}'                                 # plan view
odm render '{"look": "left"}'                                # side elevation
odm render '{"wireframe": true, "width": 1600}'              # inspect topology
odm render '{"opacity": 0.3}'                                # x-ray: see inside
odm render '{"inputs": {"t": 2.5}, "out": "/tmp/frame.png"}' # one animation moment
odm render '{"path": "parts/wheel.js", "inputs": {"radius": 12}}'  # one part alone
```

Don't read dimensions off pixels — `inspect` is exact.

## Talking with the user

ODM runs you: the user types into the viewer's Agent panel and it
arrives as a message in this conversation; your replies appear there
as you write them. There is nothing to poll and nothing to send — just
answer. Keep replies short and plain (a line or two; the panel shows
text as typed, and the rebuilt scene speaks for itself).

- Each user message carries an attachment, `odm://user-state`: a JSON
  snapshot of what the user was looking at *as they sent it* — viewer
  tab `slot` and `path`, its `inputs`, their `selection`, and the
  `camera`. Paste `camera`'s contents into `odm render` to see exactly
  what they saw. No attachment means no tab was open.
- A message that starts `[odm engine]` was written by the engine, not
  the user: a build broke (or a file's background check failed) while
  you were idle, or a change you made left something failing when your
  turn ended. The errors are attached as `odm://diagnostics`. Fix what
  it names; nobody is waiting on a reply.
- Your working status in the viewer is derived from your turn — the
  plan entry in progress, else the tool call you are running — so there
  is nothing to set or clear. The user can stop your turn from the
  panel.
- `odm` commands and edits inside the project run without asking;
  anything else may ask the user for permission first.

The viewer also logs what you do next to what you say: one line per
command you run and per file you change.

When the user refers to a part ("make *this* one longer"), the
message's `odm://user-state` `selection` has it — clicked parts appear as
`{id, name}`, in pick order (shift-click selects several); `odm
status` shows the current list on demand. To query exactly what the
user is seeing (their tab, their input values), add `"view": true` to
any scene query.
