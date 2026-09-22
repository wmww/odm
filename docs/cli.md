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

The exception is `docs`, which is engineless and textual. `--project`
stays a prefix — transport, resolved before a request exists.

Every command that looks at the scene **syncs first**: it rescans the
project's files, rebuilds what changed, then answers — so a query can
never show anything older than your latest save, and there is no
separate "sync" step to run. Each command prints a single JSON object;
exit code 0 = ok, nonzero = error. (`docs` is the exception: no engine,
and markdown rather than JSON.)

## Views

Every scene query targets a **view**: a part (default `root.js`)
built with chosen input values. The shared request fields below say
which view and how: `path` names the part; `inputs` sets any
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
Request fields every view-targeting command (inspect, render, raycast, clearance, export) shares:

- `path` (string) — the part to build (default `root.js`)
- `inputs` (object) — input values by name, e.g. `{"t": 1.5}`; plain inputs of the target become view args, everything else a view-level cascade value — a name nothing reads is an error listing the settable inputs
- `preset` (string) — apply a named bundle from the target's `meta.presets` first; explicit `inputs` override it
- `view` (true | string) — target what the user sees: `true` = the active viewer tab (its path and inputs) as the base, a string = a specific slot (`status` lists them)
- `stats` (bool) — add build stats to the response: which parts re-ran (with self-time) vs. were served from the memo cache

### status

project and session state: files, view slots with their inputs and build state, the user's active tab and selection. Reports last-published outcomes — never waits on a build, so it answers when everything else fails with a build error.

No request fields.

### inspect

the scene tree, measured exactly: names, bounds, counts — and, on the root entry, the view's interface.

- `node` (string) — one node, by the name you gave it (`"seat"`) or by index path, which starts with a slash (`"/1/0/2"`); default the root
- `depth` (number) — expand this many levels below the addressed node
- `recursive` (bool) — expand fully
- `full` (bool) — every measurement field, repeats expanded; `fields` overrides it
- `fields` (array of strings) — exactly these fields; per-node: `name`, `color`, `bounds`, `tris`, `verts`, `volume`, `area`, `position`, `rotation`, `scale`, `matrix`, `world_matrix`; view-level, on the root entry only: `description`, `inputs` (every settable input: value, type/range, default, declaration site, plain vs cascade), `presets`

### render

render a PNG; prints its path and echoes the resolved camera (`eye`/`target`/`up` + `fov` or `ortho_height`, all request fields — nudge and paste them back, unwrapped).

- `width` (number) — pixels, 16..=8192 (default 1024; per tile of a `frames` sheet, 512)
- `height` (number) — pixels, 16..=8192 (default 768; per tile of a `frames` sheet, 384)
- `frames` (array of objects) — contact sheet: one tile per entry, each an object of render fields merged over this request (`inputs` merges by key) and captioned with what it overrides — e.g. `[{"inputs": {"t": 0}}, {"inputs": {"t": 1}}]` (animation moments) or `[{"look": "top"}, {"look": "front"}, {}]` (a drafting sheet; `{}` is the default view). Tiles share one auto-fit framing (the union of all frames' bounds), so scale is comparable across the sheet; a frame's own `focus`/`eye`/`zoom` still overrides its tile. `width`/`height`/`supersample`/`out`/`view`/`stats` stay whole-sheet fields
- `out` (string) — output file (default under `.odm/renders/`); the CLI resolves it against its own cwd, and the engine refuses to overwrite project source files
- `wireframe` (bool) — edges only, in each object's own color — surfaces are not drawn
- `no_grid` (bool) — hide the ground grid
- `opacity` (number) — x-ray, 0..=1: multiplies every object's alpha, so everything turns translucent and interiors show through
- `supersample` (number) — render k× larger internally and box-downsample: anti-aliasing on demand, none by default (integer 1..=8; k×width/height must fit the GPU's texture limit)
- `look` (keyword | [x,y,z]) — `"top"`/`"bottom"`/`"front"`/`"back"`/`"left"`/`"right"` — the six axis-aligned drafting views, orthographic; or gaze along a vector, perspective (`front` looks along +y, `top` along -z); default a framed perspective overview
- `focus` (string) — frame this node's subtree (name or `/1/0/2` index path, as `inspect` addresses nodes); the rest of the scene is still drawn
- `zoom` (number) — factor on the auto-fitted distance/height: 2 = twice as close
- `ortho` (bool) — projection override; beats what `look` implies in either direction
- `eye` ([x,y,z]) — camera position; alone, it looks at the (focused) center
- `target` ([x,y,z]) — look-at point (default the framed bounds' center)
- `up` ([x,y,z]) — camera up (default `[0,0,1]`; `[0,1,0]` looking straight up/down)
- `fov` (number) — perspective field of view in degrees (default 45; the auto fit adapts to it)
- `ortho_height` (number) — orthographic view height in world units (default fits the framed bounds)

### raycast

nearest surface hit along each ray.

- `rays` (array) — rays to fire, each `{"origin": [x,y,z], "dir": [x,y,z]}` (optional `"max_dist"`); all against the request's one view, answered in order — `hits` holds `{id, name, distance, point, normal}` or `null` per ray. `id` is the mesh node's index path; `name` is that node's own name, else its nearest named ancestor's (so naming an invoked part labels hits inside it), else null

JS twin: `s.raycast(origin, dir, maxDist?)` — same query, same result shape; the CLI adds `id`/`name` per hit and maps over `rays`.

### clearance

assembly check: per pair of nodes, the signed distance between them (positive = exact gap, negative = penetration).

- `pairs` (array) — node pairs to check, each `["a", "b"]` (names or `/1/0/2` index paths, as `inspect` addresses them; each node stands for its whole subtree); all against the request's one view, answered in order. Per pair, `clearances` holds a signed `distance` — positive: the exact minimum gap, with `closest` (the two nearest points) — negative: the nodes overlap, and `separate` is a translation of the pair's second node that clears the first (its length is `-distance`, an upper bound on true penetration depth). `between` names the deciding leaf pair, and a negative result adds `overlapping`: every colliding leaf pair. The sign of a near-zero distance is float noise (exact tangency): treat `|distance|` below your own tolerance as contact — don't nudge geometry to disambiguate

JS twin: `a.clearance(b)` on Solids — same query; the numeric fields only (in JS you hold the two solids, so there is nothing to name), points as Vector3s; the CLI addresses nodes and maps over `pairs`.

### export

write the view's solids to a file — binary STL, in millimetres, world coordinates as modeled (Z-up, no recentering). Answers the file's `size_mm`, `volume_mm3`, `tris`, `bodies`, and any `warnings` worth reading (a size that suggests the wrong unit).

- `out` (string) — required: the file to write, replaced atomically; the extension picks the format — `.stl` is the only one today
- `units` (string) — what one model unit is — `mm`, `m`, `in` or `ft`; default the project's (`units` in odm.toml, itself default `mm`; `status` reports it). The file is always millimetres
- `union` (bool) — default true: fuse every solid into one valid manifold — overlaps merge, disjoint solids stay separate bodies. `false` writes each solid as-is (exact, but overlapping solids self-intersect)

### feedback

report an ODM bug or missing feature. Writes the report into the project for the user to review; they send it, or throw it away. Nothing is reported back either way.

- `title` (string) — one line: what is wrong, or what is missing
- `body` (string) — the report itself: what you did, what happened, what you expected — with the exact request and response, and any source, pasted in (there is no attachment mechanism)
- `harness` (string) — the agent harness you are running in, e.g. `Claude Code`
- `model` (string) — your model name, as exactly as you know it

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
*last-published* value (`build`: `ok` / `error` / `pending`, plus the
error message), so it still answers when the project is broken.
`stale: true` on a slot means a newer generation's answer is queued or
building — the value shown is the last one published, never masked by
an in-progress build. Also there: the project path, name and `units`
(what one model unit is, from `odm.toml`; `mm` when it doesn't say), the
part file list, whether `root.js` exists, each slot's path and
inputs, which tab is the user's active one, their current `selection`
(on the active slot), the `generation` — an internal counter that ticks
whenever a source file changes, useful only for checking that an edit
was picked up — and `health` (below).

**`health`: the whole-project failure list.** Slots only cover what's
on screen; in the background the engine also sweeps every file per
generation — a meta check of each one (syntax errors, load-time
throws, bad meta), plus a build of its default view for files whose
declared inputs all have defaults. `health` lists the failures only:
`{path, error}` per file whose last check failed, with `stale: true`
when the current generation hasn't re-evaluated it yet. Absence claims
nothing beyond "no known failure" — and a passing default view is a
canary at one specific view, not a verdict: an error can genuinely
depend on inputs, so a file can be `ok` here and still fail at a
slot's inputs (or vice versa). Files with a required no-default plain
input can't build standalone by design; for them the meta check is the
whole check.

## inspect

`odm inspect` measures the scene exactly, where a render only shows
shape.

```
odm inspect                             # whole scene, recursive, summary
odm inspect '{"node": "seat"}'          # that node, in full, children as a count
odm inspect '{"node": "seat", "recursive": true}'   # ...and its subtree
odm inspect '{"fields": ["name", "bounds"]}'        # narrow the columns instead
odm inspect '{"fields": ["inputs", "presets"]}'     # the view's interface
```

**Addressing.** A node is addressed by the `name` you gave it
(`s.name('seat')`). An index path from the root also works — it starts
with a slash (`"/0"`, `"/1/0/2"`; `""` or `"/"` is the root) — and is the
tiebreaker when a name is used more than once: the duplicate-name error
lists the matching ids, which are index paths. The slash is what tells
the two apart, so a name is never mistaken for a path (`"12"` is the node
you called `12`); a name that itself starts with `/` is reachable only by
its index path.

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
repeats. Like `tris`, `volume` and `area` are subtree totals — per-solid
sums, so overlapping siblings double-count shared space (`clearance` is
the overlap check); if any part of a subtree fails to measure, the key
is omitted rather than report a partial sum. `fields` picks exactly what
you want, and overrides `full` when both are given.

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
spelling the request accepts. Paste the object's *contents* back at
the top level — there is no `camera` request field. "Slightly to the
left" is a nudge of the echoed numbers pasted back; a user message's
`odm://user-state` speaks the same spelling, so their own view replays
verbatim.

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
odm render '{"look": [-1, 0, -0.4], "focus": "seat"}'        # frame one node
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
(the view spec, node addressing by name: hits carry `id` and the
nearest name at or above them).
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
miss, in `hits`, in request order. `id` is the index path of the node
owning the mesh; `name` is that node's own name, else its nearest named
ancestor's, else `null` — so naming a group or an invoked part labels
every hit inside it, while a name deeper down still wins. Precise probing — "what is directly
under this point", clearance along a line. For "how big / where is a
node", `inspect` is the better tool.

```
odm raycast '{"rays": [{"origin": [0, 0, 50], "dir": [0, 0, -1]},
                       {"origin": [40, 0, 8], "dir": [-1, 0, 0], "max_dist": 25}]}'
```

### clearance

The assembly self-check: "is A attached to B / do these collide". Each
pair is two nodes (names or `/1/0/2` index paths, as `inspect` addresses
them), each standing for its whole subtree; `clearances` answers per
pair, in order:

```
odm clearance '{"pairs": [["seat", "chainL"], ["seat", "chainR"]], "inputs": {"t": 1.5}}'
```

Per pair, a signed `distance`:

- **Positive** — the nodes are clear, `distance` is the exact minimum
  gap, and `closest` gives the two nearest points (`[[x,y,z],[x,y,z]]`,
  on the first and second node): where the gap is, not just how big.
- **Negative** — the nodes overlap. `separate` is a translation for the
  pair's *second* node that clears the first; its length is
  `-distance`. It is a guarantee (applying it separates the nodes) and
  an upper bound on the true penetration depth — "chainL is ~2.1 into
  the seat; move +z by 2.1" is the intended reading.

```json
{"distance": 2.5, "closest": [[…], […]], "between": ["seat", "chainL"]}
{"distance": -1.2, "separate": [0, 0, 1.2], "between": ["roof-panel", "beam-ew-1"],
 "overlapping": [["roof-panel", "beam-ew-1"], ["lacing-north", "beam-ew-2"]]}
```

`between` names the deciding leaf pair (nearest when clear, first
collider when not) and `overlapping` lists *every* colliding leaf pair,
so a group-vs-group check says what collides without bisecting by hand.
Leaves are named by their own or nearest named ancestor's name (else
index path).

**Contact:** at exact tangency the *sign* is floating-point noise, so
resting/touching solids read as `distance ≈ 0` of either sign. Treat
`|distance|` below your own tolerance as contact; do not nudge geometry
apart just to make the sign stable.

One command checks a whole assembly's contact pairs after an edit, and
`inputs` lets you check at animation extremes.

## Exporting geometry

```
odm export '{"out": "bracket.stl"}'
odm export '{"out": "arm.stl", "path": "parts/arm.js", "inputs": {"t": 0.5}}'
```

`export` writes every solid of the view — the whole scene, translucent
ones included; color is ignored — as one binary STL. To export one
piece of an assembly, export the part that builds it (`path`). The extension of
`out` picks the format; `.stl` is the only one today.

- **Millimetres.** STL carries no unit and readers assume mm, so the
  file is converted from the project's unit (`units` in `odm.toml`: `mm`
  — the default —, `m`, `in` or `ft`). `"units"` in the request overrides
  it for one export. Coordinates are otherwise exactly as modeled: Z-up,
  no recentering, so pieces exported separately stay registered.
- **`union`** (default `true`) fuses all solids into one valid manifold:
  overlapping solids merge, disjoint ones stay separate bodies. `false`
  writes each solid as-is — exact, but overlapping solids self-intersect,
  which some readers mishandle.

The response reports what was written: `path`, the resolved `units` and
`union`, `size_mm`, `volume_mm3`, `tris`, `bodies`. Read the `warnings`:
a longest side under 1 mm or over 2000 mm almost always means the wrong
unit. The file is replaced atomically, and the same view and options
always produce the same bytes.

## Talking with the user

There is no chat command. ODM runs the agent itself (over ACP — the
Agent Client Protocol), and the conversation is the channel: what the
user types into the viewer's Agent panel arrives as a message, and the
agent's replies, tool calls and plan show there as they stream. The CLI
is the agent's tool surface; it carries no messages. (`poll`, `say` and
`ack` are gone; asking for them says so.)

- **User state is sent, not sampled.** Each user message carries a
  second content block, an embedded resource `odm://user-state`: JSON
  with the viewer tab's `slot` and `path`, its `inputs`, the user's
  `selection` (`{id, name}` per clicked node, in pick order —
  shift-click selects several) and the `camera`, in the same
  `eye`/`target`/`up`/`fov` spelling `render` accepts, so pasting its
  contents into a render replays their exact view. Stamped as they hit
  Enter: the user may have moved on by the time it is read. No tab
  open, no attachment. On demand, `odm status` shows the current
  selection on the active view slot.
- **Build diagnostics are pushed.** When the set of failing things —
  view slots and files from the background sweep, each with its error —
  changes while the agent is idle, the engine sends a prompt of its
  own: text starting `[odm engine]`, with the full errors attached as
  `odm://diagnostics`. While a turn is running nothing is pushed (the
  agent's own half-done edits make transient failures, and its commands
  already report build errors); whatever still stands when the turn
  ends goes as one follow-up. Value changes only: ok→ok saves, stale
  flips and heals send nothing. After three engine prompts in a row
  with no user message between, the engine says so in the panel and
  stops forwarding until the user next speaks. Engine host warnings
  ("file watcher unavailable") ride the same prompts. `status` always
  has the current state (`views[].build`, `health`); query the view
  for error + logs in full.
- **The working status is derived**, never set: the turn running is
  the status, labelled with the plan entry in progress, else the
  running tool call. It ends when the turn does.
- **Permissions.** `odm` commands and edits inside the project run
  unasked (where the agent has a way to be told so); everything else
  asks the user in the panel, unless they picked YOLO in Agent Settings.

Agents run by hand, outside ODM, can still use the CLI — they just
have no chat.

## Reporting problems

`odm feedback` is how an ODM bug or a missing capability gets back to
the people who build ODM. It is for **ODM itself** — the engine, the
CLI, the JS API, the viewer — not for bugs in the project you are
working on, which are yours to fix. Check `odm docs` first: a
capability that exists and is documented is not a missing one.

```
odm feedback '{"title": "clearance errors on nested nodes",
               "body": "…", "harness": "Claude Code", "model": "…"}'
```

All four fields are required. `harness` and `model` are yours to fill
in — you know them; they are what makes a pattern of reports readable.

What belongs in `body`:

- the smallest reproduction you have, as code — paste the part
  source (or the few lines that matter) into the body,
- the exact request you ran and the exact response you got back,
- what you expected instead.

There is **no attachment mechanism**: a file path or a render is no use
to someone reading this on another machine, so paste the text. The
platform and the exact build are recorded for you.

The report is written into the project (`.odm/feedback/<id>.json`) and
goes no further on its own: **a human reads it in the viewer and
decides whether to send it**, and may edit it first. You will not hear
back — the response is a future release, and there is nothing to wait for.
So file it and move on with the work.
