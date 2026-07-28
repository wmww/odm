# CLI reference

The `odm` CLI talks to the running engine. Run it from anywhere inside
the project (it walks up to find the engine socket), or pass
`--project <dir>` first. Every command **syncs first** — rescans files,
rebuilds what changed, then answers — and prints a single JSON object.
Exit code 0 = ok, 1 = error.

```
odm status                    # files, generation, animation duration
odm build [--t 1.5]           # build only; errors + console logs
odm tree [--t] [--depth N]    # node ids, names, meshes, world bounds
odm inspect <node-id> [--t]   # volume, area, bounds, world matrix
odm raycast --origin 0,0,50 --dir 0,0,-1 [--t]
odm render [options]          # PNG → prints path
odm selection                 # what the user selected in the viewer
odm poll [--timeout <sec>]    # wait for messages from the user
odm say <text>                # send a message to the user
odm prompt                    # print these instructions
```

Node ids are child-index paths from the root (`""`, `0`, `0/2`); get
them from `odm tree`.

## Rendering

Options: `--t sec`, `--width/--height px` (default 1024×768),
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
odm render --t 2.5 --out /tmp/frame.png      # animation frame
```

Don't read dimensions off pixels — `tree`/`inspect`/`raycast` are exact.

## Talking with the user

The user types messages into the viewer. They queue in the engine until
you collect them with `odm poll`:

- `odm poll` blocks until at least one message is queued, then prints
  them all — `{"ok": true, "messages": [{"text": "..."}, ...]}` — and
  exits. If messages are already waiting it returns immediately.
- It also exits (nonzero) if the engine goes away, so it never hangs
  forever. `--timeout <sec>` additionally bounds the wait, exiting with
  `"messages": []` — use it if your harness limits how long a command
  may run.

**Keep a poll running in the background at all times**, including while
you work: launch `odm poll` as a background task, and whenever it exits,
act on any messages and launch it again. Messages are never lost —
anything sent while you weren't polling is delivered to the next poll —
but the viewer tells the user nobody is listening when no poll is
active, so a standing poll is what makes you reachable.

`odm say <text>` sends a message back; it appears in the viewer next to
the user's own messages. Use it to answer questions and report what you
did — the rebuilt scene speaks for itself, so keep it short. Don't use
`say` for progress narration on every edit.

When the user refers to a part ("make *this* one longer"), check
`odm selection` — clicked parts appear there as `{node, name}`, in pick
order (shift-click selects several).
