# ODM documentation

Docs for *using* ODM on a project (writing doohickeys, driving the
engine). Docs about ODM's implementation live in `notes/`.

- `prompts/` — the lean in-context layer: the markdown that
  `odm prompt` compiles in and prints for pasting into an agent's
  context. Keep it short; depth belongs in `api/`.
- `api/` — the full JS API reference, one topic per file. Start at
  `api/README.md`.

Planned (see `plans/api-versioning-infra.md`): `changes/vN.md`
migration guides, frozen `vN/` snapshots at version cuts, an
`odm docs` CLI for searching this tree, and doctests over every
example. None exist yet.

**Stability**: the current API is the unstable dev channel. It breaks
freely until v1 is cut; these docs track the live surface.
