# Agent surface policy (prompt vs docs)

Adopted 2026-08 (from plans/cli-diet.md, after the 2026-08 surface audit).
The scarce resource is the agent's context: prompt is ~3K tokens paid once
per session, but output *shape* dominates everything (the un-dieted `tree`
printed 191 KB). Priority order for cuts: output fields ≫ concepts ≫
command count.

## The policy

- **The prompt teaches only the core loop**: edit → `inspect` → `render` →
  `poll`/`say`, plus "view commands take one JSON request", `inputs`/
  `preset`, and "parts have names". After basic work the concept load
  should be: files are doohickeys; queries target a view; `inputs` sets
  inputs; parts have names; poll/say to talk.
- **Everything else lives in `odm docs`**, paid for only when consulted.
  `docs/cli.md` (topic `cli`) is the full CLI reference: every request
  field of every command (a generated section printed from the engine's
  own spec table — `requests.rs` — so it can't drift), geometry queries,
  explicit camera placement, viewer view slots, index-path addressing,
  `--project`. Cascade mechanics, memoization, generations, color spaces,
  camera math — optional depth.
- **New features default to docs-only.** Promotion into the prompt needs a
  reason on the scale of the core loop.
- Cuts are about defaults and prompt space — full capability stays
  reachable via CLI + docs, and `odm --help` lists everything (one line
  per command; fields live in docs).

## One JSON grammar (2026-08, plans/cli-json-args.md)

`odm [--project <dir>] <cmd> ['{…json}']` — the argument *is* the socket
request body, minus `cmd`. No per-command flags, no other positionals;
bare = defaults. Exceptions, deliberately: `poll --timeout/--follow`
(configures the CLI's own waiting; `--follow` never reaches the engine),
`say <free text>`, `docs` (engineless), `--project` prefix (transport).
The engine's spec table validates field names (typos list siblings,
removed commands get redirect errors) — one error path, no CLI grammar
errors beyond "that wasn't a JSON object". Wire word is `inputs`
(renamed from `set` 2026-08: it's the word meta and the interface report
use; "set" read imperative).

## Geometry queries: flat toplevel, JS parity

Kernel geometry queries (`raycast` today; distances, sections, mass
properties later — see plans/clearance.md) are plain toplevel commands
sharing the JSON grammar. Standing decisions:

- **No `query` namespace** — a prefix is a classification the agent must
  remember, costs a word per call; command count is the cheapest surface
  there is. The set-ness lives in docs (`docs/cli.md`'s "Geometry
  queries" section + JS-twin table), not grammar.
- **JS parity rule**: every kind is defined once — name, parameters,
  result shape — and exists in both surfaces: JS as a method
  (`s.raycast(origin, dir, maxDist?)`), CLI as the same-named command
  with the same result shape plus what JS gets free from object
  references (view spec; hits carry `id`/`name`). No tagged union in
  either surface — per-kind serde structs with `deny_unknown_fields`;
  don't reintroduce one "for generality".
- **Plural-native**: where mapping is natural the request takes arrays
  (`rays: […]`), all against the request's one view, answered in order.
  No heterogeneous cross-kind arrays — that's two commands, and the
  memoized build makes the second nearly free.
- `inspect` is not in the parity set — no JS twin (in JS you hold the
  object graph); simply absent from the JS-twin table.

## Standing cuts (don't reintroduce)

- No standalone `sync` command: every command syncs first; `status` is the
  answer if the rescan is all you want.
- **No `build` command** (2026-08): split into the `inspect` root entry's
  view-level fields (`"fields": ["description", "inputs", "presets"]`),
  `"stats": true` on any view command, and the one error path (build
  errors return through whichever command triggered the build, with the
  declared interface attached when meta evaluated). Input lints ride the
  `warnings` channel of every view-targeting success.
- **No `selection` command** (2026-08): `status` reports the active
  slot's selection; every poll carries it (`view.selection`).
- **No `prompt` command** (2026-08): it's a docs topic (`odm docs
  prompt`) — the prompt's standing home is the AGENTS.md marker block,
  so the command's only remaining use was inspection, which is docs.
- One "target what the user sees" knob: `"view": true` = active viewer
  tab, `"view": "<slot>"` = a named slot. There is no `--viewer-state`.
- `generation` appears only in `status` (internal consistency counter).
- Nothing agent-visible prints content hashes.
- Removed-command redirects live engine-side (`requests.rs::removed`),
  so raw-socket users get them too; `prompt` alone redirects client-side
  (its replacement needs no engine).
