# Notes index

- `architecture.md` — the system as built: project format, crate map with
  per-crate gotchas, viewer fonts/icons/tree, user↔agent chat, invariants,
  testing, seeing the viewer. Start here.
- `api-stability-and-docs.md` — agreed model for JS API versioning
  (per-file pragma, frozen conformance suites over one live
  implementation, bug-compat old versions) and versioned searchable docs
  with doctests. Build plan: `plans/api-versioning-infra.md`.
- `design-decisions.md` — why the stack/architecture is what it is: decisions
  from the 2026-07 concept review, rejected alternatives, licensing, egui
  i18n limits.
- `spike-findings.md` — measured facts from the pre-MVP spikes and stack
  verification: V8/deno_core threading rules and numbers, scheduler dedup
  policy, three→Manifold weld results, cancellation timings.
- `ecosystem-research-2026-07.md` — survey of geometry kernels, Rust viewer
  stacks, and web-viewer options (July 2026, with sources). The evidence base
  behind design-decisions.
- `build-environment.md` — machine facts; per-checkout target dirs (seeding,
  why sharing corrupts), sweeping stale artifacts, why mold/sccache stay off.
