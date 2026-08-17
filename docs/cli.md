# CLI reference

The full reference for the `odm` CLI. The short version — the core
loop — is in the agent prompt (`odm docs prompt`); everything the CLI
can do is here.

The CLI talks to a running engine over the project's unix socket. Run it
from anywhere inside the project — it walks up from cwd to the nearest
`odm.toml`, git-style, and talks to that project's engine — or pass
`--project <dir>` first, which is taken exactly as given (no walking up
from it). Start an engine with `odm run <dir> --headless` (or plain
`odm run` for the viewer).

## One grammar

```
odm [--project <dir>] <cmd> ['{…json}']
```

A command's argument is **one JSON object: the whole request**. Bare
means defaults (`odm inspect` = root view, summary tree). There are no
per-command flags and no other positionals. Single-quote the object so
the shell leaves it alone:

```
odm inspect
odm inspect '{"node": "seat"}'
odm render '{"inputs": {"t": 1.5}, "width": 640}'
odm raycast '{"rays": [{"origin": [0, 0, 50], "dir": [0, 0, -1]}]}'
```

The exceptions are the commands whose arguments aren't a structured
request: `poll` takes `--timeout <sec>`/`--follow` (they configure the
CLI's own waiting; `--follow` never reaches the engine), `say` takes
free text, and `docs` is engineless and textual. `--project` stays a
prefix — transport, resolved before a request exists.

Every command that looks at the scene **syncs first**: it rescans the
project's files, rebuilds what changed, then answers — so a query can
never show anything older than your latest save, and there is no
separate "sync" step to run. Each command prints a single JSON object;
exit code 0 = ok, nonzero = error. (`docs` is the exception: no engine,
and markdown rather than JSON.)

## Views

Every scene query targets a **view**: a doohickey (default `root.js`)
built with chosen input values. The shared request fields below say
which view and how: `path` names the doohickey; `inputs` sets any
input (plain inputs of the target become view args, anything else —
declared cascade inputs, or names read by invoked descendants — becomes
a view-level cascade value; a name nothing reads is an error listing
the settable inputs); `preset` applies a named value bundle from the
target's `meta.presets` first; `view` adopts what the user is looking
at as the base, with `inputs`/`preset` overriding on top.

CLI views are one-off: they build (memoized) but do not publish, so
they never change what the viewer shows.

## Commands

<!--- BEGIN GENERATED COMMAND REFERENCE --->
Request fields every view-targeting command (inspect, render, raycast, clearance) shares:

- `path` (string) — the doohickey to build (default `root.js`)
- `inputs` (object) — input values by name, e.g. `{"t": 1.5}`; plain inputs of the target become view args, everything else a view-level cascade value — a name nothing reads is an error listing the settable inputs
- `preset` (string) — apply a named bundle from the target's `meta.presets` first; explicit `inputs` override it
- `view` (true | string) — target what the user sees: `true` = the active viewer tab (its path and inputs) as the base, a string = a specific slot (`status` lists them)
- `stats` (bool) — add build stats to the response: which doohickeys re-ran (with self-time) vs. were served from the memo cache

### status

project and session state: files, view slots with their inputs and build state, the user's active tab and selection. Reports last-published outcomes — never waits on a build, so it answers when everything else fails with a build error.

No request fields.

### inspect

the scene tree, measured exactly: names, bounds, counts — and, on the root entry, the view's interface.

- `node` (string) — one node, by the name you gave it (`"seat"`) or by index path (`"1/0/2"`); default the root
- `depth` (number) — expand this many levels below the addressed node
- `recursive` (bool) — expand fully
- `full` (bool) — every measurement field, repeats expanded
- `fields` (array of strings) — exactly these fields; per-node: `name`, `color`, `bounds`, `tris`, `verts`, `volume`, `area`, `position`, `rotation`, `scale`, `matrix`, `world_matrix`; view-level, on the root entry only: `description`, `inputs` (every settable input: value, type/range, default, declaration site, plain vs cascade), `presets`

### render

render a PNG; prints its path and echoes the resolved camera (`eye`/`target`/`up` + `fov` or `ortho_height` — nudge and paste back).

- `width` (number) — pixels, 16..=8192 (default 1024; per tile of a `frames` sheet, 512)
- `height` (number) — pixels, 16..=8192 (default 768; per tile of a `frames` sheet, 384)
- `frames` (array of objects) — contact sheet: one tile per entry, each an object of render fields merged over this request (`inputs` merges by key) and captioned with what it overrides — e.g. `[{"inputs": {"t": 0}}, {"inputs": {"t": 1}}]` (animation moments) or `[{"look": "top"}, {"look": "front"}, {}]` (a drafting sheet; `{}` is the default view). Tiles share one auto-fit framing (the union of all frames' bounds), so scale is comparable across the sheet; a frame's own `focus`/`eye`/`zoom` still overrides its tile. `width`/`height`/`supersample`/`out`/`view`/`stats` stay whole-sheet fields
- `out` (string) — output file (default under `.odm/renders/`); the CLI resolves it against its own cwd, and the engine refuses to overwrite project source files
- `wireframe` (bool) — edges only, in each object's own color — surfaces are not drawn
- `no_grid` (bool) — hide the ground grid
- `opacity` (number) — x-ray, 0..=1: multiplies every object's alpha, so everything turns translucent and interiors show through
- `supersample` (number) — render k× larger internally and box-downsample: anti-aliasing on demand, none by default (integer 1..=8; k×width/height must fit the GPU's texture limit)
- `look` (keyword | [x,y,z]) — `"top"`/`"bottom"`/`"front"`/`"back"`/`"left"`/`"right"` — the six axis-aligned drafting views, orthographic; or gaze along a vector, perspective (`front` looks along +y, `top` along -z); default a framed perspective overview
- `focus` (string) — frame this node's subtree (name or index path, as `inspect` addresses nodes); the rest of the scene is still drawn
- `zoom` (number) — factor on the auto-fitted distance/height: 2 = twice as close
- `ortho` (bool) — projection override; beats what `look` implies in either direction
- `eye` ([x,y,z]) — camera position; alone, it looks at the (focused) center
- `target` ([x,y,z]) — look-at point (default the framed bounds' center)
- `up` ([x,y,z]) — camera up (default `[0,0,1]`; `[0,1,0]` looking straight up/down)
- `fov` (number) — perspective field of view in degrees (default 45; the auto fit adapts to it)
- `ortho_height` (number) — orthographic view height in world units (default fits the framed bounds)

### raycast

nearest surface hit along each ray.

- `rays` (array) — rays to fire, each `{"origin": [x,y,z], "dir": [x,y,z]}` (optional `"max_dist"`); all against the request's one view, answered in order — `hits` holds `{id, name, distance, point, normal}` or `null` per ray

JS twin: `s.raycast(origin, dir, maxDist?)` — same query, same result shape; the CLI adds `id`/`name` per hit and maps over `rays`.

### clearance

assembly check: per pair of nodes, do they overlap, and at least how far apart are they.

- `pairs` (array) — node pairs to check, each `["a", "b"]` (names or index paths, as `inspect` addresses them; each node stands for its whole subtree); all against the request's one view, answered in order — `clearances` holds `{overlap, gap_lower_bound}` per pair. `overlap` is exact (shared volume); `gap_lower_bound` is from bounding boxes, so 0 means "close or touching", not necessarily contact

JS twin: `a.clearance(b)` on Solids — same result shape; the CLI addresses nodes and maps over `pairs`.

### poll

wait for messages the user typed in the viewer.

- `timeout` (number) — seconds to wait before answering with no messages (default: wait until a message arrives or the engine stops)

### say

send a message to the user.

- `text` (string) — the message

<!--- END GENERATED COMMAND REFERENCE --->

## Errors

Build errors (with the JS stack and `console.log` output) come back
through whichever command triggered the build, exit nonzero — there is
no separate error query. When the target's `meta` evaluated (meta can
succeed while `build()` throws), the error response still carries the
declared `inputs` and `presets` — what's needed to fix a wrong input.
Unknown request fields are errors that list the valid ones.

On success, input lints (conflicting fall-through defaults, unread
cascade values, plain-shadows-cascade, type conflicts) arrive on the
`warnings` channel of any view-targeting response.

## status

The one command that never builds: it reports each view slot's
*last-published* outcome (`build`: `ok` / `error` / `building` /
`pending`, plus the error message), so it still answers when the
project is broken. Also there: the project path and name, the doohickey
file list, whether `root.js` exists, each slot's path and inputs, which
tab is the user's active one, their current `selection` (on the active
slot), and the `generation` — an internal counter that ticks whenever a
source file changes, useful only for checking that an edit was picked
up.

## inspect

`odm inspect` measures the scene exactly, where a render only shows
shape.

```
odm inspect                             # whole scene, recursive, summary
odm inspect '{"node": "seat"}'          # that part, in full, children as a count
odm inspect '{"node": "seat", "recursive": true}'   # ...and its subtree
odm inspect '{"fields": ["name", "bounds"]}'        # narrow the columns instead
odm inspect '{"fields": ["inputs", "presets"]}'     # the view's interface
```

**Addressing.** A node is addressed by the `name` you gave it
(`s.name('seat')`). An index path from the root also works — `"0"`,
`"1/0/2"`, `""` for the root — and is the tiebreaker when a name is
used more than once: the duplicate-name error lists the matching ids,
which are index paths.

**Scope.** Bare `inspect` is a whole-scene recursive overview; naming a
node asks about that node, with its children as a count. `depth`
expands N levels below the addressed node; `recursive` expands fully.

**Detail.** Every entry has `id`, `name`, world `bounds` and `tris`
**for its whole subtree** — the root's bounds are the model's overall
extent, a group's are the group's. Runs of identical consecutive
siblings collapse into one entry with `repeat: N`: their ids run on
consecutively from the one shown, and they differ only in placement.
`full` adds `verts`, `volume`, `area` (world-space, computed on demand)
and the node's own `position`/`rotation`/`scale`, and expands the
repeats. `fields` picks exactly what you want.

**The view's interface.** The root entry (`""` *is* the view) carries
the view-level fields: `description`, `inputs` — every settable name in
one flat list, each entry with its current value, type/range, default,
where it is declared, and whether it is a plain arg or a cascade
value — and `presets`. `odm inspect '{"fields": ["inputs", "presets"]}'`
is the one answer to "what can I set here", and a *failed* build still
reports the declared inputs and presets next to the error.

## render

`odm render` renders a PNG and prints its path.

`opacity` (0..1) is x-ray: it multiplies every object's alpha, so
everything turns translucent and interiors show through.
`supersample` (k) renders k× larger internally and box-downsamples:
anti-aliasing on demand; there is none by default. The internal
k×width/height must fit the GPU's texture limit.

**Camera: one parameter set, no modes.** A camera is target + gaze
direction + distance + up + projection; every parameter is
independently either given or defaulted, and the defaults frame the
scene (or the `focus` node) so the model always fits. With no camera
fields you get a framed perspective overview from a deliberately
skewed direction. Any single field alone is meaningful: `eye` alone
looks at the framed center from there, `fov` alone adapts the fit to
the lens, `eye` + `ortho` fits the ortho height for you.

- `look` — `"top"`, `"bottom"`, `"front"`, `"back"`, `"left"`,
  `"right"`: the six axis-aligned drafting views, orthographic (that's
  why you type the word); or a vector to gaze along, perspective.
  `front` looks along +y, `top` along −z. `ortho` overrides the implied
  projection in either direction: `{"look": "top", "ortho": false}` is
  a perspective top-down.
- `focus` — frame one node's subtree (addressed as `inspect` addresses
  nodes); the rest of the scene is still drawn.
- `zoom` — factor on the fitted distance/height: 2 = twice as close.
- `eye`, `target`, `up`, `fov`, `ortho_height` — exact placement,
  each usable alone; the others stay defaulted.

Over-determined combinations are errors, not precedence rules:
`eye`+`look`, `eye`+`zoom`, `zoom`+`ortho_height`, `fov` on an
orthographic camera, `ortho_height` on a perspective one.

Every response echoes the **resolved camera** — `camera`:
`eye`/`target`/`up` plus `fov` or `ortho` + `ortho_height`, the same
spelling the request accepts. "Slightly to the left" is a nudge of the
echoed numbers pasted back; poll snapshots speak the same spelling, so
the user's own view replays verbatim.

**Contact sheets: `frames`.** One render, many tiles: `frames` is an
array of partial requests, each merged over the base request (shallow
per field; `inputs` merges by key) and rendered as one captioned tile,
row-major in a near-square grid. Motion reads far better side by side
than across separate files, and one sheet is one image to open. Every
frame is explicit — no ranged sampling; write out the values you want.

- Any per-frame field goes: `inputs` (animation moments, parameter
  sweeps), `preset`, `path`, camera fields, `wireframe`/`opacity`.
  `{}` is a tile of the base request unchanged. `width`/`height` (the
  per-*tile* size, default 512×384 for sheets), `supersample`, `out`,
  `view`, and `stats` shape the whole sheet and stay top-level.
- **Tiles share one framing**: the default fit is computed from the
  union of every frame's bounds, so scale is comparable across tiles
  and motion doesn't wobble. A frame's own `focus`/`eye`/`zoom` still
  overrides its tile, per parameter as usual.
- Each tile is captioned with its overrides (`t=0.75`, `look=top`).
  The response carries the shared resolved `camera`, plus a per-frame
  one for any frame that overrode camera fields.
- Every frame is an ordinary view build (individually memoized); a
  failing frame fails the sheet naming the frame, e.g.
  `frames[2] (t=1.5): …`.

```
odm render                                                   # framed overview
odm render '{"look": "top"}'                                 # plan view
odm render '{"look": "left"}'                                # side elevation
odm render '{"look": [-1, 0, -0.4], "focus": "seat"}'        # frame one part
odm render '{"wireframe": true, "width": 1600}'              # inspect topology
odm render '{"opacity": 0.3}'                                # x-ray: see inside
odm render '{"inputs": {"t": 2.5}, "out": "/tmp/frame.png"}' # one animation moment
odm render '{"path": "parts/wheel.js", "inputs": {"radius": 12}}'  # one part alone
odm render '{"eye": [60, -80, 40], "target": [0, 0, 10], "fov": 30}'  # exact framing
odm render '{"frames": [{"inputs": {"t": 0}}, {"inputs": {"t": 0.75}}, {"inputs": {"t": 1.5}}]}'  # motion sheet
odm render '{"frames": [{"look": "top"}, {"look": "front"}, {"look": "left"}, {}]}'  # drafting sheet
```

Don't read dimensions off pixels — `inspect` is exact.

## Geometry queries

Kernel geometry queries are plain toplevel commands sharing the JSON
grammar — no `query` namespace. Each kind is defined once — name,
parameters, result shape — and exists in both surfaces: in JS as a
method on the object, on the CLI as the command of the same name with
the same result shape plus what JS gets free from object references
(the view spec, node addressing by name: hits carry `id`/`name`).
Where mapping is natural the request takes arrays — all against the
request's one view spec, answered in order, one build. `inspect` is
not in this set: it has no JS twin (in JS you hold the object graph).

| CLI | JS twin |
|-----|---------|
| `raycast` | `s.raycast(origin, dir, maxDist?)` (`odm docs queries`) |
| `clearance` | `a.clearance(b)` (`odm docs queries`) |

### raycast

Fires each of `rays` and reports the nearest surface hit per ray:
`{id, name, distance, point, normal}` (world-space), or `null` for a
miss, in `hits`, in request order. Precise probing — "what is directly
under this point", clearance along a line. For "how big / where is a
part", `inspect` is the better tool.

```
odm raycast '{"rays": [{"origin": [0, 0, 50], "dir": [0, 0, -1]},
                       {"origin": [40, 0, 8], "dir": [-1, 0, 0], "max_dist": 25}]}'
```

### clearance

The assembly self-check: "is A attached to B / do these collide". Each
pair is two nodes (names or index paths, as `inspect` addresses them),
each standing for its whole subtree; `clearances` answers per pair, in
order:

```
odm clearance '{"pairs": [["seat", "chainL"], ["seat", "chainR"]], "inputs": {"t": 1.5}}'
```

Per pair: `{"overlap": bool, "gap_lower_bound": n}`. `overlap` is exact
(the solids share volume — exact surface contact is not overlap).
`gap_lower_bound` only bounds the gap from below (bounding boxes): a
**positive** value guarantees the parts are at least that far apart —
misplaced-part bugs read as a surprising gap here — while 0 just means
the boxes touch (contact, interpenetration, and interlocking parts with
real clearance all read 0). There is no signed distance.

One command checks a whole assembly's contact pairs after an edit, and
`inputs` lets you check at animation extremes.

## Talking with the user

The user types messages into the viewer's chat panel. They queue in the
engine until collected with `odm poll`:

- `odm poll` blocks until at least one message is queued, then prints
  them all — `{"ok": true, "messages": [{"text": "..."}, ...]}` — and
  exits. If messages are already waiting it returns immediately. Each
  message carries a `view` field: a snapshot of what the user was
  looking at **when they sent it** (viewer tab path, its input values,
  their selection, and the camera — in the same `eye`/`target`/`up`/
  `fov` spelling `render` accepts, so pasting it into a render replays
  their exact view). Stamped at send time: the user may have moved on
  by the time you poll.
- It also exits (nonzero) if the engine goes away, so it never hangs
  forever. `--timeout <sec>` additionally bounds the wait, exiting with
  `"messages": []` — use it if your harness limits how long a command
  may run.
- `--follow` never exits: it prints one compact JSON line per batch (the
  same object, one per line) and keeps waiting. For a harness that
  surfaces each line of a long-running command, this is one standing
  command instead of a relaunch per message.
- Interrupting a poll (Ctrl+C, a killed background task) loses nothing:
  a message is only retired once the poll that took it has printed it,
  so anything it didn't get to goes back in the queue for the next one.
  The flip side is that a poll killed at exactly the wrong moment can
  make one message arrive twice — if the same text turns up again
  immediately, it is the same instruction, not a second one.

`odm say <text>` sends a message back; it appears in the viewer next to
the user's own messages. Everything after `say` is the message — no
quoting rules.

When the user refers to a part ("make *this* one longer"), the
message's `view.selection` has it — clicked parts appear as
`{id, name}`, in pick order (shift-click selects several). On demand,
`odm status` shows the current list on the active view slot.
