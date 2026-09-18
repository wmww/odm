# ODM documentation

Docs for *using* ODM on a project (writing doohickeys, driving the
engine). Docs about ODM's implementation live in `notes/`.

- `prompts/` — the lean in-context layer: the markdown compiled into
  `odm-prompt`, which `odm docs prompt` prints and the engine keeps
  current between the markers in a project's `AGENTS.md`/`CLAUDE.md`.
  Keep it short; depth belongs in `api/`.
- `api/` — the full JS API reference, one topic per file. Start at
  `api/README.md`. Served by `odm docs` (compiled into the binary, so
  it always matches the engine; works with no engine running).
- `cli.md` — the full CLI reference (`odm docs cli`); `prompts/cli.md`
  keeps only the core loop.
- `changes/` — API migration guides, one per version hop (none yet).
- `versioning.md` — what API versions promise and how cuts happen.

At a version cut the live tree is copied to a frozen `docs/vN/`
snapshot (`odm docs --api N`); no snapshots exist yet. Every fenced
`js` example in this tree is doctested — see
`crates/odm-build/tests/doctests.rs`.

**Stability**: the current API is the unstable dev channel. It breaks
freely until API 1 is cut; these docs track the live surface.
