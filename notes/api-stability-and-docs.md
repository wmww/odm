# JS API stability & docs plan

Status: discussed with user 2026-07-29; direction agreed below except where
marked open. Not yet implemented.

## Why

The JS API must be stable once ready: projects are long-lived and agents
shouldn't be forced to migrate whole projects (error-prone) on engine
upgrades. CLI/engine internals need no such promise — agents are
re-prompted each session. Current API is NOT frozen yet (user explicit).

## Versioning model

- **Per-file API version**, chosen by the file itself via a static pragma
  readable at sync time without evaluating the module. Syntax (user
  decided): `//! odm v1`, `//! odm v2`, …; `//! odm unstable` for the
  permanent dev channel (always the current surface, no promises, breaks
  freely; in-repo examples live here). Cutting vN copies the then-current
  unstable surface/docs/suite to vN and freezes them.
- **Single integer versions** (Rust-editions style): the pragma gates
  breaking changes only. Feature additions land silently and are exposed
  to every file of that major — user explicitly wants no feature-hiding
  for old files. Safe in JS: a file shadowing a name in its own isolate
  keeps its shadow. Rejected X.Y min-featureset (Go-style): unenforced
  minor rots; its only payoff (clear "engine too old" error) is had more
  cheaply via good unknown-API errors. Pragma grammar can grow an
  optional minor later if lagging-engine scenarios become real.
  Fits the architecture for free: each doohickey gets its own isolate, so
  version = which framework snapshot to instantiate.
- **One live implementation, not frozen copies.** Rust-editions model:
  engine ops + core stay singular and current; each version is a thin JS
  shim over them. The frozen artifact per version is its **conformance
  test suite** (+ docs), not code. Anything may change as long as every
  version's frozen suite passes.
- **Old versions stay bug-compatible by default** (user decision —
  reversed my earlier Rust-style stance). A core bugfix that would change
  a stamped version's behavior gets a compat shim preserving the old
  behavior, unless we deliberately decide the fix applies there too.
  "Frozen" suites may still be touched to: add tests, port to new test
  infra, and (when decided) assert a bugfix in an old version.
- Suites assert **semantics, not bytes** (volume/bounds/raycast within
  epsilon, image-diff tolerance) — Manifold upgrades change exact
  triangulation legitimately.
- Pragma **required** once v1 exists (missing → error with hint);
  "default = latest" would reintroduce break-on-upgrade.
- Cross-version `ctx.invoke` works via the engine-mediated boundary
  (JSON + handles); that protocol is engine-owned, additive-only,
  versioned separately from the API surface.
- Vendored THREE subset is pinned per API version (part of the surface).
  Deterministic Math.random PRNG likewise per-version.
- Cadence: batch breaking changes into deliberate infrequent cuts —
  every version is permanent surface (shim + suite + docs).

## Sequencing

Current API = the unstable channel, explicitly breakable. Build real
test projects on it now, accept churn (agents do mechanical migrations),
let usage shape the API. Cut v1 when it stops moving under load; the
test projects' assertions seed the v1 conformance suite. Do not
stabilize from theory. User confirmed: projects + agent feedback rounds
before stabilizing anything.

## Docs

- Layout (done 2026-07-29, user picked): everything under `docs/` —
  `docs/prompts/` is the lean in-context layer (~100-line cheat sheet,
  compiled into `odm prompt`; moved from top-level `prompts/`), and
  `docs/api/` is the full reference, one topic per file (written; see
  `docs/api/README.md` for the index).
- Full reference searched on demand via CLI (`odm docs <query>`, grep +
  section extraction to start) so agents pull detail without filling
  context — CLI not built yet.
- Version cut snapshots `docs/vN/`; frozen versions' docs are frozen
  (typo fixes ok). CLI `--api N`, default latest.
- **Doctests**: every docs example extracted and run in CI against its
  version. This enforces both docs-freshness and API stability; examples
  double as conformance tests.
- **Migration guides**: one file per hop, named `docs/changes/vN.md`
  (covers v(N-1) → vN; user picked this naming). Bullets only, mechanical
  before→after per breaking change. CLI serves the concatenated path
  (`odm docs changes 1 4`) so an agent updating a file gets exactly the
  deltas. Guides freeze once written; before/after snippets doctested
  under their respective versions.

## Decided

- Single integer versions, no X.Y (user confirmed).
- Old versions live indefinitely; revisit only if a shim becomes
  burdensome (user confirmed).
- Suites are append-only within a live major as features are added;
  "frozen" precisely = existing assertions never weakened or removed.
- Plan for building the infra: `plans/api-versioning-infra.md`
  (explicitly does not include cutting v1).

## Open questions

- Whether odm.json (params/animation) semantics fall under the API
  version or stay a separate additive-forever format (leaning latter).
  User wants a broader discussion of odm.json and how we think about
  projects vs files — pending.
- Project-level metadata recording a target engine version, so an agent
  opening a project touched by a newer engine gets nudged to update
  ("your coworker edited this on a newer version"). Deferred — can be
  added backwards-compatibly later, nothing baked into project files.
