# CLI: one JSON grammar, consolidated commands

## Why
The CLI is already a JSON translator: `parse_opts` builds the request map the
socket wants and flags are a lossy sugar over it. Structured requests
(contact-sheet frames, per-frame cameras — `render-frames.md`) don't fit a
flat flag grammar, and every flag is a concept the agent pays for. Merge the
two grammars into the one that was underneath all along: the request body.
Token cost of JSON ceremony is accepted deliberately — this CLI is
agent-first, and agents write JSON all day (`gh api`, `curl`).

The 2026-08 review pass widened the scope from "same commands, JSON args" to
a consolidated command list (below).

## Grammar
`odm [--project <dir>] <cmd> ['{…json}']`

The JSON object is the socket request body, minus the command. Within a
command, no flags and no other positionals — mixing grammars would mean
keeping both. Bare means defaults (`odm inspect` = root view, summary tree).

The rule: **view-targeting commands take one JSON request; everything else
stays bare, flags, or text.**

## The command list

    odm status                # session/user state; never blocks, answers when broken
    odm inspect ['{…}']       # the built scene: tree, measurements; root fields carry the view interface
    odm raycast '{…}'         # geometry queries with JS twins, one toplevel
    odm clearance '{…}'       #   command per kind; this set grows
    odm render  ['{…}']       # PNG
    odm poll / say            # user channel (flags / free text, unchanged)
    odm docs / run            # reference (incl. prompt topic) / engine

- JSON-in-scope: `inspect`, `render`, and every geometry query — the
  commands that share the view-spec shape.
- Out of scope: `status` is bare. `poll` keeps `--timeout`/`--follow` — its
  args configure the CLI's own waiting behavior (`--follow` never reaches
  the engine), not a structured request. `say` keeps free text (prose; its
  no-quoting-rules design is the point); `docs` is engineless and textual;
  `--project` stays a prefix (transport — resolved before a request exists).

### Removed commands
- **`build`** — split three ways:
  - Interface ("what can I set here") → view-level fields on `inspect`'s
    root entry (`""` *is* the view): `description`, `inputs`, `presets`.
    Full detail (type/range, default, declaration site, plain vs cascade) in
    the values; input lints on the normal warnings channel. Bare `inspect`
    output is unchanged — the facet is opt-in:
    `odm inspect '{"fields": ["inputs", "presets"]}'`. The prompt's
    core-loop section carries that one-liner (it replaces teaching `build`).
  - Build stats (what re-ran, self-time vs memo) → opt-in `"stats": true`
    request field on any view-targeting command.
  - Errors → the one error path below, plus per-slot state in `status`.
- **`selection`** — gone; `status`'s active-view info absorbs the current
  selection. (Poll's snapshot only shows it as of the last message; status
  is the on-demand "what is the user looking at right now".)
- **`prompt`** — becomes a docs topic (`odm docs prompt`). The engine keeps
  the prompt current in the AGENTS.md marker block itself, so the standalone
  command's only remaining use is inspection — that's docs. Drops out of
  `--help` automatically (docs topics are listed by bare `odm docs`).

## Geometry queries: toplevel, one command per kind
Kernel geometry queries — `raycast`, `clearance` (see `clearance.md`), and
whatever comes later (distances, sections, mass properties, containment…) —
are plain toplevel commands sharing the JSON grammar. Deliberately no
`query` namespace: a prefix is a classification the agent must remember
(why `query clearance` but plain `inspect`?), costs a word per call, and
command count is the cheapest surface there is (`notes/agent-surface.md` —
fields ≫ concepts ≫ commands). The set-ness lives in docs, not grammar:
`docs/cli.md` groups them in one "geometry queries" section, each kind
naming its JS twin, and `--help` groups the one-liners the same way.

- **JS parity rule**: every kind is defined once — name, parameters, result
  shape — and exists in both surfaces: JS as a method
  (`solid.raycast(origin, dir)`, `a.clearance(b)`), CLI as the command of
  the same name with the same result shape, plus what JS gets free from
  object references (view spec, node addressing by name).
  The API exposes kinds as separate functions and the CLI as separate
  commands, so the tagged union exists in *neither* place — don't
  reintroduce one "for generality"; per-kind serde structs with
  `deny_unknown_fields` are the point.
- **Plural-native kinds**: where mapping is natural the request takes
  arrays — `raycast` takes `rays: […]`, `clearance` takes `pairs: […]` —
  all against the request's one view spec, answered in order. One build,
  one generation, view spec written once ("check every contact pair after
  this edit" is one command). Parity is about the unit query; the CLI form
  maps over it. No heterogeneous query arrays — a cross-kind mix is two
  commands, and the memoized build makes the second nearly free.
- `inspect` is not in the parity set — no JS twin (in JS you hold the
  object graph); it's the agent's window into the tree. Nothing in the
  grammar marks that; it's simply absent from the JS-twin table in docs.

## One error path
Any view-targeting command whose build fails returns the build error (JS
stack, `console.log` output) *as* its response, exit nonzero — no separate
error query. When the target's `meta` export evaluated (meta can succeed
while `build()` throws), the error response still carries the declared
inputs and presets — what's needed to fix a wrong input.

serde `deny_unknown_fields` catches typos; invest in messages (unknown field
lists its siblings, wrong-shape says what was expected). One error path
instead of CLI-grammar errors + engine errors.

## status
The command that must still answer when the project is broken — everything
view-targeting can fail with a build error; status is the agent's footing.
Viewless: project path/name, files, generation, and the user's state — per
active view slot: path, inputs, which tab is active, current selection, and
build state (ok / error message / building). It reports last-published
outcomes and never waits on builds.

## Design
- **Rename `set` → `inputs`** on the wire while every call site changes
  anyway. It's the word meta and the interface report already use; "set"
  read imperative, as if persisting something.
- The CLI keeps only transport: project discovery, socket, pretty-printing,
  the poll ack handshake, resolving `out` against its cwd (a client-side
  pass over the body). `parse_opts`/`ArgKind` shrink to poll's two flags.
- **Schema as spec**: describe each request with the profiled-JSON-Schema
  machinery doohickey inputs already use; `docs/cli.md`'s per-command
  reference prints from the same source that validates, so docs can't
  drift. `odm --help` shrinks to a command list, one line each; `odm docs`
  gets one section per geometry query.
- Prompt: core-loop examples switch to JSON form
  (`odm render '{"inputs": {"t": 1.5}}'`); concept count drops — "view
  commands take one JSON request" replaces every flag ever taught.

## Direction: one view-spec shape
A view spec `{path, inputs, preset?, view?}` is latent in four shapes today:
request bodies, viewer tab persistence, `poll`'s attached view snapshot,
`meta.presets`. Converge them on one schema — plausibly also the JS invoke
API's argument object, so CLI, protocol, persistence, and code API say the
same thing the same way. Requests first; align the others opportunistically,
not as v1 scope.

## Notes
- inspect's args are already structural (`fields` list, `inputs` object) and
  will grow (plural `nodes`, name filters) — JSON is right even for the
  hot-loop command; the frequent call is bare anyway.
- Shell quoting: single-quote the object; values containing `'` are rare
  here — one line in docs, not a design driver.
- `render-frames.md`, `render-camera.md`, `clearance.md` build on this; land
  this first so nothing ever grows a flag spelling.
- On implementation, record in `notes/agent-surface.md`: the JS parity rule
  for geometry queries (flat toplevel, no `query` namespace), and the
  removals (`build`, `selection`, standalone `prompt`) as standing cuts.
