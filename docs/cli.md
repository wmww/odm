# CLI reference

The full reference for the `odm` CLI. The short version — the core
loop — is in the agent prompt (`odm prompt`); everything the CLI can do
is here.

The CLI talks to a running engine over the project's unix socket. Run it
from anywhere inside the project — it walks up from cwd to the nearest
`odm.toml`, git-style, and talks to that project's engine — or pass
`--project <dir>` first, which is taken exactly as given (no walking up
from it). Start an engine with `odm run <dir> --headless` (or plain
`odm run` for the viewer).

Every command that looks at the scene **syncs first**: it rescans the
project's files, rebuilds what changed, then answers — so a query can
never show anything older than your latest save, and there is no
separate "sync" step to run. Each command prints a single JSON object;
exit code 0 = ok, nonzero = error. (`prompt` and `docs` are the
exceptions: no engine, and markdown rather than JSON.)

```
odm status                    # files, views, generation, project name
odm build   [<view options>]          # settable inputs, presets, build stats, logs
odm inspect [<node>] [<view options>] [--depth N] [--recursive]
            [--full | --fields a,b,c]        # measure the scene
odm render  [<view options>] [camera + output options]   # PNG → prints path
odm raycast --origin x,y,z --dir x,y,z [<view options>]  # nearest hit
odm selection                 # what the user selected in the viewer
odm poll [--timeout <sec>] [--follow]   # wait for messages from the user
odm say <text>                # send a message to the user
odm prompt                    # print the agent instructions
odm docs [<topic>]            # this reference (list topics when bare)
odm docs search <pattern>     # grep the reference, whole sections out
odm docs changes <from> <to>  # API migration guides, concatenated
```

Build errors (with JS stacks and `console.log` output) come back
through whichever command triggered the build; there is no separate
error query.

## Views

Every scene query targets a **view**: a doohickey (default `root.js`)
built with chosen input values.

- `<path>` as the first positional argument (or `--path <p>`) names the
  doohickey; without one, `root.js`.
- `--set name=value` sets any input. Values parse as JSON, falling back
  to plain strings (`--set t=1.5`, `--set finish=painted`,
  `--set 'size=[10,20,5]'`). Plain inputs of the target become view
  args; anything else (declared cascade inputs, or names read by
  invoked descendants) becomes a view-level cascade value. A name
  nothing reads is an error listing the settable inputs.
- `--preset <name>` applies a named value bundle from the target's
  `meta.presets` first; explicit `--set` wins over it.
- `--view` targets **what the user sees** instead: bare, the user's
  active viewer tab (its path *and* its input values) becomes the base,
  with `--set`/`--preset` overriding on top. `--view <slot>` adopts a
  specific tab; slots are listed in `odm status` → `views`. Naming a
  `<path>` too keeps only the tab's cascade values.

CLI views are one-off: they build (memoized) but do not publish, so
they never change what the viewer shows.

## status

Project overview: the project path and name, the list of doohickey
files, whether `root.js` exists, the active view slots (path + set
inputs + which is the user's active tab), and the current `generation` —
an internal counter that ticks whenever a source file changes, useful
only for checking that an edit was picked up.

## build

`odm build [<view options>]` is the one answer to "what can I set here": the
target's description and presets, plus `inputs` — every settable name in
one flat list, each entry with its current value, type/range, default,
where it is declared, and whether it is a plain arg or a cascade value.
`warnings`/`errors` carry input lints (conflicting defaults, unread
cascade values, plain-shadows-cascade). `stats` shows which doohickeys
actually re-ran (with per-file self-time) vs. were served from the memo
cache. A *failed* build still reports the target's declared inputs and
presets next to the error.

## inspect

`odm inspect` measures the scene exactly, where a render only shows
shape.

```
odm inspect                   # whole scene, recursive, summary
odm inspect seat              # that part, in full, children as a count
odm inspect seat --recursive  # ...and its subtree
odm inspect --fields name,bounds     # narrow the columns instead
```

**Addressing.** A node is addressed by the `name` you gave it
(`s.name('seat')`). An index path from the root also works — `0`,
`1/0/2`, `""` for the root — and is the tiebreaker when a name is used
more than once: the duplicate-name error lists the matching ids, which
are index paths.

**Scope.** Bare `inspect` is a whole-scene recursive overview; naming a
node asks about that node, with its children as a count. `--depth N`
expands N levels below the addressed node; `--recursive` expands fully.

**Detail.** Every entry has `id`, `name`, world `bounds` and `tris`
**for its whole subtree** — the root's bounds are the model's overall
extent, a group's are the group's. Runs of identical consecutive
siblings collapse into one entry with `repeat: N`: their ids run on
consecutively from the one shown, and they differ only in placement.
`--full` adds `verts`, `volume`, `area` (world-space, computed on
demand) and the node's own `position`/`rotation`/`scale`, and expands
the repeats. `--fields a,b,c` picks exactly what you want from:
`name, color, bounds, tris, verts, volume, area, position, rotation,
scale, matrix, world_matrix`.

## render

`odm render [<view options>] [options]` renders a PNG and prints its path.

Output options: `--width`/`--height` in pixels (default 1024×768,
16..=8192), `--out file.png` (default under `.odm/renders/`; the engine
refuses to overwrite project source files), `--wireframe` (edges only,
in each object's own color — surfaces are not drawn), `--no-grid`.

Camera, auto-framed (the model always fits the frame):

- default: isometric perspective;
- `--direction x,y,z` looks along that vector (`0,0,-1` = top view);
- `--ortho` makes either of the above orthographic.

Camera, explicit placement (when framing must be exact):

- `--eye x,y,z --target x,y,z` place the camera; `--up x,y,z` defaults
  to `0,0,1`;
- `--fov <deg>` perspective field of view (default 45), or
  `--ortho --ortho-height <h>` for an orthographic view `h` world units
  tall.

```
odm render                                   # framed isometric
odm render --direction 0,0,-1 --ortho        # top view (plan)
odm render --direction -1,0,0 --ortho        # side elevation
odm render --wireframe --width 1600          # inspect topology
odm render --set t=2.5 --out /tmp/frame.png  # one moment of an animation
odm render parts/wheel.js --set radius=12    # view one part alone
odm render --eye 60,-80,40 --target 0,0,10 --fov 30   # exact framing
```

Don't read dimensions off pixels — `inspect` is exact.

## raycast

`odm raycast --origin x,y,z --dir x,y,z [<view options>]` fires one ray and
reports the nearest surface hit: `{id, name, distance, point, normal}`
(world-space), or `null` for a miss. Precise probing — "what is directly under this
point", clearance along a line. For "how big / where is a part",
`inspect` is the better tool; from inside doohickey code, use
`solid.raycast()` (see `odm docs queries`).

## Talking with the user

The user types messages into the viewer's chat panel. They queue in the
engine until collected with `odm poll`:

- `odm poll` blocks until at least one message is queued, then prints
  them all — `{"ok": true, "messages": [{"text": "..."}, ...]}` — and
  exits. If messages are already waiting it returns immediately. The
  response's `view` field is a snapshot of what the user was looking at
  (viewer tab path, its input values, their selection) — context for
  the words next to it.
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

`odm selection` lists what the user has clicked in the viewer as
`{id, name}` pairs, in pick order (shift-click selects several). The
same list rides along in every poll's `view.selection`.
