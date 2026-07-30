# ODM documentation

Docs for *using* ODM on a project (writing doohickeys, driving the
engine). Docs about ODM's implementation live in `notes/`.

- `prompts/` — the lean in-context layer: the markdown that
  `odm prompt` compiles in and prints for pasting into an agent's
  context. Keep it short; depth belongs in `api/`.
- `api/` — the full JS API reference, one topic per file. Start at
  `api/README.md`. Served by `odm docs` (compiled into the binary, so
  it always matches the engine; works with no engine running).
- `changes/` — API migration guides, one per version hop (none yet).
- `versioning.md` — what API versions promise and how cuts happen.

At a version cut the live tree is copied to a frozen `docs/vN/`
snapshot (`odm docs --api N`); no snapshots exist yet. Every fenced
`js` example in this tree is doctested — see
`crates/odm-build/tests/doctests.rs`.

**Stability**: the current API is the unstable dev channel. It breaks
freely until v1 is cut; these docs track the live surface.
