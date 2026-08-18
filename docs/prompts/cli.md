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
odm poll --follow             # stream user messages forever; park under a per-line watcher
odm poll [--timeout <sec>]    # one-shot fallback: wait for messages, exit
odm say <text>                # send a message to the user
odm say --task <text>         # set the live "working on..." status
odm say --done [<text>]       # clear it (+ optionally send a message)
odm docs [<topic>]            # full reference (list topics when bare)
odm docs search <pattern>     # grep the reference, whole sections out
```

This page is the short version; `odm docs cli` is the full one — every
request field of every command, explicit camera placement, viewer view
slots, index paths, `--project`. (`poll` and `say` are the exceptions
to the JSON grammar: a wait bound and free text.)

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

The user types messages into the viewer. They queue in the engine until
you collect them with `odm poll`:

- `odm poll` blocks until at least one message is queued, then prints
  them all — `{"ok": true, "messages": [{"text": "..."}, ...]}` — and
  exits. If messages are already waiting it returns immediately. Each
  message carries a `view` snapshot of what the user was looking at
  when they sent it: viewer tab path, its input values, their
  selection, and the camera — paste `view.camera`'s contents into
  `odm render` to see exactly what they saw.
- It also exits (nonzero) if the engine goes away, so it never hangs
  forever. `--timeout <sec>` additionally bounds the wait, exiting with
  `"messages": []` — use it if your harness limits how long a command
  may run.
- Every response also carries `builds` (each view slot's build state:
  ok/error/pending, with the error) and `health` (files whose
  background check failed) — so a broken build reaches you with the
  next poll, viewed or not. Engine warnings arrive as messages marked
  `"from": "engine"`.
- `--follow` never exits: it prints one compact JSON line per batch (the
  same object, one per line) and keeps waiting, including a line
  whenever a build or health value changes (a slot turning red, a
  heal). It survives engine restarts: on engine death it prints
  `{"engine": "down"}`, reconnects when the engine returns, and prints
  `{"engine": "back"}` — no retry wrapper needed, and starting it
  before the engine is up parks it the same way.
- Interrupting a poll (Ctrl+C, a killed background task) loses nothing:
  a message is only retired once the poll that took it has printed it,
  so anything it didn't get to goes back in the queue for the next one.
  The flip side is that a poll killed at exactly the wrong moment can
  make one message arrive twice — if the same text turns up again
  immediately, it is the same instruction, not a second one.

**Stay reachable at all times**, including while you work: at the
start of the session, park one `odm poll --follow` under a mechanism
that *notifies you on each output line* (Claude Code: the `Monitor`
tool with `persistent: true`). A mechanism that only notifies when the
task exits does not work — `--follow` never exits, so its lines pile
up unread while the viewer tells the user someone is listening
(Claude Code: Bash `run_in_background` is exit-notify only; a shell
`&` orphans the process entirely). If exit-notify background tasks
are all your harness has, background a one-shot `odm poll` instead —
it exits at the first batch, so the notification wakes you — and
relaunch it each time you act on one. With no background mechanism at
all, fall back to foreground `odm poll --timeout <sec>` between steps.
Every line `--follow` prints is worth waking for: it only emits on
messages, build breaks/heals, health changes, and engine down/back.
Messages are never lost — anything sent while nothing was polling is
delivered to the next poll, and the viewer shows the user which of
their messages have reached you — but it also tells them nobody is
listening when no poll is active, so the standing poll is what makes
you reachable.

`odm say <text>` sends a message back; it appears in the viewer next to
the user's own messages. Use it to answer questions and report results
— a line or two; the rebuilt scene speaks for itself.

**Show what you're working on.** The viewer is the user's only window
onto you, and a silent one reads as a dead one. Sending a message puts
the status line up on its own — it reads "Processing" from the moment
the user hits Enter — and it is yours from there: replace it with what
you are actually doing, `odm say --task <text>` — a few words, present
progressive ("resizing connectors"). Update it whenever you move to a
new step (it's one line in the viewer, updated in place — cheap, so err
on the side of updating).

**Always clear the status before you stop.** `odm say --done <one-line
result>` posts the message and clears it; `odm say --done` alone just
clears it. A status left standing tells the user you are still working
when you have finished and gone — so clear it before your last word,
every time, including when you stop early or give up. There is one
status at a time; setting another replaces it, and it never expires on
its own — a standing `task` is echoed in every say/poll response, so if
you see one that no longer matches what you're doing, clear or replace
it.

The viewer also logs what you do next to what you say: one line per
command you run and per file you change.

When the user refers to a part ("make *this* one longer"), the
message's `view.selection` has it — clicked parts appear as
`{id, name}`, in pick order (shift-click selects several); `odm
status` shows the current list on demand. To query exactly what the
user is seeing (their tab, their input values), add `"view": true` to
any scene query.
