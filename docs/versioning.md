# API versioning

How the JS API makes its stability promise: projects are long-lived, and
an engine upgrade must never force a migration on code that didn't ask
for one.

## The pragma

Every doohickey names the API version it targets in its leading
comments:

```js skip
//! odm unstable
```

Valid values are `unstable` and, once cut, `v1`, `v2`, …. The pragma is
read at sync time without running the module; an unknown version fails
that file's build with the supported list. Today a missing pragma means
`unstable`; once v1 exists it becomes an error (defaulting to "latest"
would reintroduce break-on-upgrade).

Versions are per *file*, and files of different versions coexist in one
project: `ctx.invoke` crosses versions freely (the boundary is JSON +
geometry handles, owned by the engine and additive-only).

## The unstable channel

`unstable` is the permanent dev channel: always the current surface, no
promises, breaks freely and silently. In-repo examples live here
forever — they are the development feedback loop. External projects
should move to stamped versions once those exist.

## What a stamped version promises

Cutting vN freezes *behavior*, not code:

- **Semantics, not bytes.** A vN file keeps building with the same
  meaning: volumes, bounds, hits within epsilon. Exact triangulation may
  change (kernel upgrades legitimately do that).
- **Bug-compatible by default.** A core bugfix that would change a
  stamped version's behavior gets a compat shim preserving the old
  behavior there, unless we deliberately decide the fix applies. Code
  written against the bug keeps working.
- **Features arrive, breaks don't** (Rust-editions style). New API lands
  in every version where it isn't a break; the pragma gates breaking
  changes only. A file that shadows a new name in its own isolate keeps
  its shadow.
- **Versions live indefinitely.** Nothing is deprecated-then-removed;
  revisit only if a shim ever becomes genuinely burdensome.

There is one live implementation, not frozen copies: engine ops and the
framework core stay singular and current, and each stamped version is a
thin JS shim over them. The frozen artifact per version is its
**conformance suite** (`tests/conformance/vN/`) plus its docs snapshot —
anything may change inside as long as every version's suite passes.

## Suites are append-only

A version's conformance suite may always gain tests and be ported to new
runner infrastructure. Existing assertions are never weakened or
removed. Asserting a bugfix in a stamped version (i.e. changing what its
suite demands) is a deliberate per-case decision, the exception to
bug-compatibility above.

## Docs and migration guides

- The live docs tree (`docs/api/`, served by `odm docs`) tracks
  unstable. A cut copies it to `docs/vN/`, frozen (typo fixes ok);
  `odm docs --api N` reads the snapshot.
- Each cut writes `docs/changes/vN.md`: mechanical before → after
  bullets for every breaking change since v(N-1).
  `odm docs changes 1 4` concatenates the exact path an agent needs to
  migrate a file.
- Every fenced example everywhere is doctested under its tree's version
  (`crates/odm-build/tests/doctests.rs`), including migration-guide
  before/after snippets under their respective versions.

## Cadence

Breaking changes are batched into deliberate, infrequent cuts — every
version is permanent surface (shim + suite + docs). Don't stabilize
from theory: v1 happens when the API stops moving under real project
load.

## Version-cut checklist (cutting vN)

1. Add `ApiVersion::V(N)` to `crates/odm-js/src/version.rs::SUPPORTED`
   and give it a manifest under `framework/versions/vN/` (starts as the
   then-current unstable surface; bare `'odm'`/`'three'` resolution per
   version lives in `crates/odm-js/src/snapshot.rs`).
2. Copy `tests/conformance/unstable/` → `tests/conformance/vN/` and
   register the channel with the runner
   (`crates/odm-engine/src/conformance.rs`). The copy is now frozen per
   the rules above; the unstable suite keeps evolving.
3. Copy the live docs (`docs/api/` + relevant top-level files) →
   `docs/vN/`; wire `odm docs --api N` to it; teach the doctest runner
   to run that snapshot's examples under vN.
4. Write `docs/changes/vN.md` (empty-but-present for v1).
5. For v1 only: flip missing-pragma from "unstable" to an error
   (`crates/odm-build/src/sources.rs`), and update every in-repo example
   and doc snippet to carry an explicit pragma.
6. Update `docs/prompts/` to tell agents to stamp new files with the
   newest version (new files should not target unstable outside this
   repo).
7. From now on, run the vN suite forever; behavior-affecting core
   changes need vN shims (`framework/versions/vN/`) instead of vN
   breakage.
