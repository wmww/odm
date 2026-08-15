# CLI reference

The `odm` CLI talks to the running engine. Run it from anywhere inside
the project — it walks up from cwd to the nearest `odm.toml` and talks to
that project's engine — or pass `--project <dir>` first, which is taken
exactly as given (no walking up from it). Every command that looks at the scene **syncs
first** — rescans files, rebuilds what changed, then answers — and prints
a single JSON object. Exit code 0 = ok, nonzero = error. (`prompt` and
`docs` are the exceptions: no engine, and markdown rather than JSON.)

```
odm status                    # files, generation, project name
odm build [<path>] [--set name=value ...] [--preset <name>]
                              # settable inputs, presets, build stats
odm inspect [<node>] [--path <p>] [--set ...] [--depth N] [--recursive]
            [--full | --fields a,b,c]        # measure the scene
odm raycast --origin 0,0,50 --dir 0,0,-1 [--path <p>] [--set ...]
odm render [<path>] [--set ...] [options]    # PNG → prints path
odm selection                 # what the user selected in the viewer
odm poll [--timeout <sec>] [--follow]   # wait for messages from the user
odm say <text>                # send a message to the user
odm prompt                    # print these instructions
odm docs [<topic>]            # full API reference (list topics when bare)
odm docs search <pattern>     # grep the reference, whole sections out
```

Every scene query targets a **view**: a doohickey (default `root.js`)
built with its declared input defaults. `--set name=value` sets any
input — values parse as JSON, falling back to plain strings (`--set
t=1.5`, `--set finish=painted`, `--set 'size=[10,20,5]'`); a typo'd
name is an error listing the settable inputs. `--preset <name>`
applies a preset from the target's meta first.

`odm build` is the one answer to "what can I set here": the target's
description and presets, plus `inputs` — every settable name, one
entry with its current value, type/range, default, and where it is
declared. Its `stats` show which doohickeys actually re-ran (with
per-file time) vs. were served from the memo cache. A failed build
still reports the target's declared inputs next to the error.

## Inspecting the scene

`odm inspect` is the measuring tool: it answers "what is in here" and
"how big is this" exactly, where a render only shows you shape.

```
odm inspect                   # whole scene, recursive, summary
odm inspect seat              # that part, in full, children as a count
odm inspect seat --recursive  # ...and its subtree
odm inspect --fields name,bounds     # narrow the columns instead
```

A node is addressed by the `name` you gave it (`s.name('seat')`); an
index path from the root (`0`, `1/0/2`, `""` = root) works too, and is
the tiebreaker when a name is used more than once — the error lists the
matching ids. Bare `inspect` is a whole-scene overview; naming a node
asks about that node, so it comes back in full detail with its children
as a count. `--depth N`/`--recursive` set how far to expand.

Every entry has `id`, `name`, world `bounds` and `tris` **for its whole
subtree** — so the root's bounds are the model's overall extent, and a
group's are the group's. Runs of identical siblings collapse into one
entry with `repeat: N`: their ids run on consecutively from the one
shown, and they differ only in placement. `--full` adds `verts`,
`volume`, `area` (world-space, on demand) and the node's own
`position`/`rotation`/`scale`, and expands the repeats. `--fields`
picks exactly what you want from `name, color, bounds, tris, verts,
volume, area, position, rotation, scale, matrix, world_matrix`.

## Rendering

Options: `--width/--height px` (default 1024×768),
`--out file.png` (default under `.odm/renders/`), `--wireframe` (edges
only, in each object's own color — surfaces are not drawn), `--no-grid`,
`--ortho`, `--direction x,y,z` (auto-framed view from that direction;
default isometric), or explicit `--eye x,y,z --target x,y,z [--up x,y,z]
[--fov deg | --ortho-height h]`.

```
odm render                                   # framed isometric
odm render --direction 0,0,-1 --ortho        # top view (plan)
odm render --direction -1,0,0 --ortho        # side elevation
odm render --wireframe --width 1600          # inspect topology
odm render --set t=2.5 --out /tmp/frame.png  # one moment of an animation
odm render parts/wheel.js --set radius=12    # view one part alone
```

Don't read dimensions off pixels — `inspect` is exact.

## Talking with the user

The user types messages into the viewer. They queue in the engine until
you collect them with `odm poll`:

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

**Stay reachable at all times**, including while you work: if your
harness can watch a long-running command's output line by line, park
`odm poll --follow` under it once at the start of the session;
otherwise launch `odm poll --timeout <sec>` as a background task and
relaunch it whenever it exits, acting on any messages it printed.
Messages are never lost — anything sent while nothing was polling is
delivered to the next poll, and the viewer shows the user which of
their messages have reached you — but it also tells them nobody is
listening when no poll is active, so a standing poll is what makes you
reachable.

`odm say <text>` sends a message back; it appears in the viewer next to
the user's own messages. Use it to answer questions and report what you
did — the rebuilt scene speaks for itself, so keep it short. Don't use
`say` for progress narration on every edit.

When the user refers to a part ("make *this* one longer"), the poll's
`view.selection` (or `odm selection`) has it — clicked parts appear as
`{id, name}`, in pick order (shift-click selects several). To query
exactly what the user is seeing (their tab, their input values), add
`--viewer-state` to any scene query.
