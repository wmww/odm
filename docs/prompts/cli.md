# CLI reference

The `odm` CLI talks to the running engine. Run it from anywhere inside
the project (it walks up to find the engine socket), or pass
`--project <dir>` first. Every command that looks at the scene **syncs
first** — rescans files, rebuilds what changed, then answers — and prints
a single JSON object. Exit code 0 = ok, nonzero = error. (`prompt` and
`docs` are the exceptions: no engine, and markdown rather than JSON.)

```
odm status                    # files, generation, project name
odm build [<path>] [--set name=value ...] [--preset <name>]
odm tree [<path>] [--set ...] [--depth N]
odm inspect <node-id> [--path <p>] [--set ...]
odm raycast --origin 0,0,50 --dir 0,0,-1 [--path <p>] [--set ...]
odm render [<path>] [--set ...] [options]    # PNG → prints path
odm interface [<path>]        # a file's description, input schemas, presets
odm selection                 # what the user selected in the viewer
odm poll [--timeout <sec>]    # wait for messages from the user
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
applies a preset from the target's meta first. The `inputs` field of a
build response lists what is settable (the fall-through report).

Node ids are child-index paths from the root (`""`, `0`, `0/2`); get
them from `odm tree`.

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

Don't read dimensions off pixels — `tree`/`inspect`/`raycast` are exact.

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
- Interrupting a poll (Ctrl+C, a killed background task) loses nothing:
  a message is only retired once the poll that took it has printed it,
  so anything it didn't get to goes back in the queue for the next one.
  The flip side is that a poll killed at exactly the wrong moment can
  make one message arrive twice — if the same text turns up again
  immediately, it is the same instruction, not a second one.

**Keep a poll running in the background at all times**, including while
you work: launch `odm poll` as a background task, and whenever it exits,
act on any messages and launch it again. Messages are never lost —
anything sent while you weren't polling is delivered to the next poll,
and the viewer shows the user which of their messages have reached you —
but it also tells them nobody is listening when no poll is active, so a
standing poll is what makes you reachable.

`odm say <text>` sends a message back; it appears in the viewer next to
the user's own messages. Use it to answer questions and report what you
did — the rebuilt scene speaks for itself, so keep it short. Don't use
`say` for progress narration on every edit.

When the user refers to a part ("make *this* one longer"), the poll's
`view.selection` (or `odm selection`) has it — clicked parts appear as
`{node, name}`, in pick order (shift-click selects several). To query
exactly what the user is seeing (their tab, their input values), add
`--viewer-state` to any scene query.
