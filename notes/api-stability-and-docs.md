# JS API stability & docs

The user/agent-facing contract lives in `docs/versioning.md` (pragma,
stamped-version promises, suite rules, the version-cut checklist). This
note holds the rationale and the implementation map. Infrastructure
built 2026-07-29 (was `plans/api-versioning-infra.md`); NO stable
version exists — the API is the freely-breaking `unstable` channel
until it stops moving under real project load (user: build test
projects and run agent feedback rounds before stabilizing anything).

## Decisions and why (user-confirmed 2026-07-29)

- **Per-file pragma** `//! odm <version>`, single integer versions
  (Rust-editions style): the pragma gates breaking changes only;
  features land in every version where they aren't a break (user
  explicitly wants no feature-hiding for old files — safe because a
  file shadowing a new name in its own isolate keeps its shadow).
  Rejected X.Y min-featureset: unenforced minor rots; its one payoff
  (clear "engine too old" error) is had cheaper via good unknown-API
  errors. Pragma grammar can grow a minor later. The same `//!` block
  also carries the doohickey's prose description (first line = summary)
  — see `plans/views-and-metadata.md` phase 1.
- **One live implementation, not frozen copies**: engine ops + core
  stay singular; each stamped version is a thin JS shim. The frozen
  artifact per version is its conformance suite + docs.
- **Bug-compatible by default** in stamped versions (user decision):
  behavior-changing core fixes get compat shims there unless we
  deliberately decide otherwise.
- Suites assert semantics within epsilon, never bytes (Manifold
  upgrades change triangulation legitimately). Append-only within a
  version; existing assertions never weakened.
- Pragma required once v1 exists (missing → error with hint);
  "default = latest" would reintroduce break-on-upgrade. Until then,
  missing = unstable.
- Cross-version `ctx.invoke` works via the engine-mediated boundary
  (JSON + handles), additive-only, versioned separately.
- Old versions live indefinitely; revisit only if a shim becomes
  burdensome.
- odm.json does not fall under the API version — it was deleted
  entirely (2026-07-29, implemented): params/animation are per-doohickey
  declared inputs, `odm.toml` is the project marker, `root.js` the
  default-view convention. Doohickey `meta` (inputs/presets) IS part of
  the versioned API surface (`ctx.get`, `ctx.invoke(path, args,
  provides)`).

## Implementation map (all built, tested)

- **Pragma parsing**: `crates/odm-js/src/version.rs` (`ApiVersion`,
  `SUPPORTED`, `parse_pragma` — only `//!` lines are pragma
  candidates, plain `//` comments never; parsed at sync time in
  `odm-build/src/sources.rs`, stored as `Source::api:
  Result<ApiVersion, String>`; a bad pragma fails only that file's
  build (`FailureKind::Version` → CLI kind `bad-version`), never the
  sync — a broken scratch file must not take the project down).
- **One snapshot, per-isolate surface selection**:
  `framework/versions/<v>` manifests register installers in
  `__odmVersions`; `run_build` executes
  `__odmVersions[v].install(globalThis)` before the doohickey loads;
  bare `'odm'`/`'three'` imports resolve per version
  (`odm-js/src/snapshot.rs`). One snapshot per version does NOT work —
  see "V8 constraints" below.
- **Test-only version** `test` (surface diff: `odm.apiProbe`) behind
  the odm-js cargo feature `test-api-version`, enabled by
  dev-dependencies only; release engines reject the id. Keeps routing,
  coexistence, and cross-version invoke exercised before v1:
  `odm-build/tests/versions.rs`.
- **Conformance suite**: `tests/conformance/unstable/*.js` (+ dirs for
  multi-file/params tests), declarative `export const checks` (volume/
  area/bounds/raycast/error/console, per-check `t`); format and
  semantics in `tests/conformance/README.md`. Runner:
  `crates/odm-engine/src/conformance.rs` (unit-test module — it needs
  crate-private query helpers). Grow it with every feature and bug.
- **`odm docs`**: `crates/odm-cli/src/docs.rs`, whole `docs/` tree
  `include_dir`'d into the binary. Topics, `search` (whole markdown
  sections out), `changes <from> <to>`, `--api N` (rejected until
  snapshots exist). `docs/changes/` exists, empty.
- **Doctests**: `crates/odm-build/tests/doctests.rs` runs every fenced
  ```js block under docs/ (` ```js skip` opts out, ` ```js error="…"`
  expects failure; fragments get a build(ctx) wrapper; `invoke('…')`
  targets get stub doohickeys). Docs examples were made
  self-contained to pass — keep new examples runnable.
- In-repo examples carry explicit `//! odm unstable` pragmas.

## V8 constraints discovered (2026-07-29, deno_core 0.408)

Pinned by `odm-js/tests/multi_snapshot.rs` (two `#[ignore]`d tests
reproduce the aborts — run individually to re-verify on V8 upgrades):

- Structurally different snapshot blobs cannot coexist in one process:
  V8 seeds a process-wide read-only heap from the first blob used;
  deserializing a different shape dies on external-reference indexes.
  This kills the original snapshot-per-version design and is why all
  versions share ONE snapshot with per-isolate install.
- Snapshot creation while any other thread executes JS aborts the
  process (the shared read-only heap is mutated during creation).
  Same-thread nesting (creation under a suspended isolate) is fine.
  Consequence for tests: one `JsEnv` per test binary, built before
  builds run (OnceLock pattern everywhere).

## Open questions

- Project-level metadata recording a target engine version (nudge
  agents on engine skew). Deferred — can be added
  backwards-compatibly later.
