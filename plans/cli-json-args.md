# CLI: one JSON argument, no flags

## Why
The CLI is already a JSON translator: `parse_opts` builds the request map the
socket wants and flags are a lossy sugar over it. Structured requests
(contact-sheet frames, per-frame cameras — `render-frames.md`) don't fit a
flat flag grammar, and every flag is a concept the agent pays for. Merge the
two grammars into the one that was underneath all along: the request body.
Token cost of JSON ceremony is accepted deliberately — this CLI is
agent-first, and agents write JSON all day (`gh api`, `curl`).

## Grammar
`odm [--project <dir>] <cmd> ['{…json}']`

The JSON object is the socket request body, minus `cmd`. No flags, no other
positionals — mixing grammars would mean keeping both.

- In scope: every engine command — `status`, `build`, `render`, `inspect`,
  `raycast`, `selection`, `poll`. All current flags/positionals become
  fields (`path`, `node`, `view`, `depth`, `timeout`, …).
- Out of scope: `say` keeps free text (prose, its no-quoting-rules design is
  the point); `prompt`/`docs` are engineless and keep their textual forms;
  `--project` stays a prefix (transport — resolved before a request exists).
- `poll`'s `follow` becomes a body field the CLI strips before sending
  (client-side loop; the engine never sees it, same as today).

## Design
- **Rename `set` → `inputs`** on the wire while every call site changes
  anyway. It's the word meta and the build report already use; "set" read
  imperative, as if persisting something.
- The CLI keeps only transport: project discovery, socket, pretty-printing,
  the poll ack handshake, resolving `out` against its cwd (a client-side
  pass over the body). `parse_opts`/`ArgKind` delete.
- **Errors are the grammar now.** serde `deny_unknown_fields` already
  catches typos; invest in messages (unknown field lists its siblings,
  wrong-shape says what was expected). One error path instead of
  CLI-grammar errors + engine errors.
- **Schema as spec**: describe each request with the profiled-JSON-Schema
  machinery doohickey inputs already use; `docs/cli.md`'s per-command
  reference prints from the same source that validates, so docs can't
  drift. `odm --help` shrinks to a command list, one line each.
- Prompt: core-loop examples switch to JSON form
  (`odm render '{"inputs": {"t": 1.5}}'`); concept count drops — "engine
  commands take one JSON request" replaces every flag ever taught.

## Direction: one view-spec shape
A view spec `{path, inputs, preset?, view?}` is latent in four shapes today:
request bodies, viewer tab persistence, `poll`'s attached view snapshot,
`meta.presets`. Converge them on one schema — plausibly also the JS invoke
API's argument object, so CLI, protocol, persistence, and code API say the
same thing the same way. Requests first; align the others opportunistically,
not as v1 scope.

## Notes
- Shell quoting: single-quote the object; values containing `'` are rare
  here — one line in docs, not a design driver.
- `render-frames.md` and `render-camera.md` build on this; land this first
  so they never grow flag spellings.
