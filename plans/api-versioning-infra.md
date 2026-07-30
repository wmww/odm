# Plan: JS API versioning + docs infrastructure

Context: notes/api-stability-and-docs.md (the agreed model). Executing this
plan does NOT create a stable v1 — we stay on the wildly unstable dev
channel for some time. This builds the machinery so that cutting v1 later
is routine.

## Version channel model (proposal)

`//! odm unstable` = the permanent dev channel: always tracks the current
surface, no promises, breaks freely. Stamped versions are `v1`, `v2`, …
Cutting vN = copying the then-current unstable surface manifest, docs, and
suite to vN and freezing them per the contract. In-repo examples stay on
`unstable` forever (they are the dev feedback loop); external projects use
stamped versions. This avoids ever renaming/retiring an identifier.

## Phase 1 — pragma + version registry

- Pragma: `//! odm unstable` (later `//! odm v1`, `//! odm v2`, …) in the
  first comment block of a doohickey. Parsed at sync time
  (`crates/odm-build/src/sources.rs`, where sources are content-hashed)
  without evaluating the module; stored on the source record. Missing
  pragma = unstable for now; flips to an error at the v1 cut.
  Unknown version → error listing supported versions.
- Version registry in `odm-js`: version → framework bundle → `JsEnv`
  snapshot (`crates/odm-js/src/snapshot.rs` currently builds exactly one
  process-wide snapshot, ~40 ms). Build snapshots lazily per version
  actually used; isolate creation picks the snapshot by the file's pragma.
- Keep the multi-version path exercised before v1 exists: a test-only
  version id (compiled in under a cfg/test feature, rejected in release)
  with one deliberate surface difference, used by integration tests to
  prove routing, coexistence, and cross-version `ctx.invoke`.

## Phase 2 — framework layering for per-version assembly

- Restructure `framework/` into core modules plus a per-version surface
  manifest (e.g. `framework/versions/unstable.js` assembling the exposed
  `odm` surface; later `framework/versions/vN/` holds that version's shims
  over the core). For now the unstable manifest is simply the whole
  current surface — mechanism only, no real shims.
- `JsEnv::new(version)` builds the snapshot from the manifest. Vendored
  THREE subset and the deterministic PRNG are part of the per-version
  bundle.

## Phase 3 — conformance suite infra

- `tests/conformance/unstable/*.js`: test doohickeys, each carrying its
  pragma.
  Assertions are semantic, never byte-exact: volume/area/bounds/raycast
  within epsilon, plus optional golden renders compared with image-diff
  tolerance (Manifold upgrades legitimately change triangulation).
- Checks format: each test file has a second export, e.g.
  `export const checks = [...]`, declarative (`{volume: [1234.5, 0.1]}`,
  raycast probes, expected build errors, console output). Runner drives a
  headless engine over the suite and evaluates checks via query APIs.
- The unstable suite is mutable and grows with every feature and
  every bug found — it is the seed of the future v1 suite. From v1 on,
  stamped suites follow the contract: append tests freely, port to new
  test infra, weaken/remove assertions never; asserting a bugfix in a
  stamped version is a deliberate per-case decision (default is
  bug-compatibility with the original).

## Phase 4 — docs tree + `odm docs` CLI

- DONE (2026-07-29): `docs/api/*.md` full reference written, one topic
  per file (index in `docs/api/README.md`); `prompts/` moved to
  `docs/prompts/` and stays the short in-context layer. Remaining: point
  prompts at `odm docs` for depth once that CLI exists.
- `docs/changes/vN.md`: migration guide for v(N-1) → vN. Bullets only,
  mechanical before→after per breaking change. None exist yet; the
  directory and CLI support do.
- Embed docs in the binary like the prompts (`crates/odm-cli/src/prompt.rs`
  pattern) so `odm docs` works from any project dir and always matches the
  engine build.
- CLI surface:
  - `odm docs` — list topics
  - `odm docs <topic>` — dump one topic file
  - `odm docs search <pattern>` — grep with section extraction
  - `odm docs changes <from> <to>` — concatenate `changes/v{from+1..to}.md`
  - `--api N` — read from the `docs/vN/` snapshot (until v1 exists, only
    the live tree is valid)
- On a version cut, the live tree is copied to `docs/vN/`; snapshots are
  frozen (typo fixes ok).

## Phase 5 — doctests in CI

- Extractor pulls fenced `js` blocks from `docs/` (api + prompts) and runs
  each under the appropriate API version. Blocks containing
  `export default` run as-is; bare fragments are auto-wrapped in a
  standard `build(ctx)` prelude; ```js skip``` opts out. Default
  assertion: builds without error; optional annotations pin expected
  values or expected error text.
- Run for the live tree always; for `docs/vN/` snapshots under version N
  once those exist. Migration-guide before/after snippets run under their
  respective versions.

## Phase 6 — versioning policy doc

- `docs/versioning.md`: the user/agent-facing contract — pragma, what a
  stamped version promises (semantics, not bytes; bug-compatible by
  default), the dev channel, change files, suite append-only rules.
- Includes the version-cut checklist so cutting v1 later is mechanical:
  stamp surface manifest, copy docs tree, freeze suite, create
  `changes/vN.md`, flip pragma-required-if-missing, update prompts.

## Non-goals (this plan)

- No v1 cut, no stability promise — unstable remains freely breakable.
- No real shims (mechanism + test-only version exercises the path).
- No docs snapshots yet; `--api` plumbing may sit unused until v1.
- No project-level engine-version nudge (deferred; can be added to
  project metadata backwards-compatibly later — see notes open items).

## Order & effort

Phases 1–2 go together (engine work, the only invasive part). Phase 3 can
start immediately after and grow indefinitely. Phases 4–5 are independent
of 1–2 and could even land first. Phase 6 is an hour of writing once the
rest is real.
