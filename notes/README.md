# Notes index

- `architecture.md` — the system as built: project format, crate map with
  per-crate gotchas, viewer fonts/icons/tree, the managed agent panel (ACP
  client, config, transcript, diagnostics push), feedback, STL export,
  invariants, testing (what earns a test here, the suites, the opt-in lanes, CI), seeing
  the viewer. Start here.
- `api-stability-and-docs.md` — versioning (JS API + the integer engine
  version): the rationale behind
  `docs/versioning.md` and the implementation map (pragma, one-snapshot
  version routing, conformance suite, `odm docs`, doctests — all built
  2026-07-29; no stable version cut yet). Read before touching the API
  surface.
- `agent-surface.md` — the prompt-vs-docs policy for the agent-facing
  surface (prompt = core loop only, depth in `odm docs`, and the two
  deliberate exceptions: `feedback`, project units), the one-JSON-grammar CLI, the geometry-query parity rules, the render camera
  parameter set, and the standing CLI cuts. Read before adding
  commands, request fields, or response fields.
- `design-decisions.md` — why the stack/architecture is what it is: decisions
  from the 2026-07 concept review plus later dated ones (colors, "user
  state: sent, not sampled"), rejected alternatives, licensing, egui
  i18n limits.
- `web-export.md` — `odm export --web` as built: the bundler/transformer,
  the odm-web wasm host and its executor seam, runtime.js semantics,
  stamp/template lookup, the wasm toolchain setup, drift caveats, and how
  to test an export. Read before touching framework module syntax, ops,
  or anything under crates/odm-export / crates/odm-web.
- `spike-findings.md` — measured facts from the pre-MVP spikes and stack
  verification: V8/deno_core threading rules and numbers, scheduler dedup
  policy, three→Manifold weld results, cancellation timings.
- `ecosystem-research-2026-07.md` — survey of geometry kernels, Rust viewer
  stacks, and web-viewer options (July 2026, with sources). The evidence base
  behind design-decisions.
- `build-environment.md` — machine facts; per-checkout target dirs (seeding,
  why sharing corrupts), sweeping stale artifacts, why mold/sccache stay off,
  the CI image/cache and reproducing a lane with podman, per-lane facts.
- `agent-integration-research-2026-09.md` — ACP as the agent-agnostic
  protocol for an ODM-managed agent panel, which agents speak it and how,
  and the (paused) Anthropic Agent SDK billing change that makes the
  risk we accepted, plus live spike results against claude-agent-acp and
  codex-acp (steering, permissions, replay, embedded context). Evidence
  base for the managed agent panel (architecture.md "Talking to the
  agent").
