# Notes index

- `architecture.md` — the system as built: project format, crate map, invariants, version-pin policies, MVP acceptance status. Start here.
- `mvp-progress.md` — MVP execution log: what landed per phase + implementation gotchas/decisions discovered along the way (op2 quirks, kernel API facts, known gaps).
- `design-considerations.md` — pre-implementation architecture decisions, recommended stack, risks, and open-question resolutions from the initial concept review.
- `ecosystem-research-2026-07.md` — survey of geometry kernels, Rust viewer stacks, and web-viewer options (researched July 2026, with sources).
- `stack-verification-2026-07-22.md` — web-verified fact-check of the stack claims (Manifold/manifold-csg, deno_core/rusty_v8, egui/wgpu/CI); includes the isolate-threading refutation.
- `spike-scheduler-findings.md` — spike 0a: dedup policy analysis (wait-for-in-flight + wait-graph cycle detection), duplication measurements.
- `spike-three-isolate-findings.md` — spike 0b: three.js subset in deno_core isolates (snapshot approach, timings, LIFO isolate-nesting rule).
- `spike-manifold-interop-findings.md` — spike 0c: three generators → Manifold weld results, booleans determinism, cancellation timings, build gotchas.

The MVP plan (`plans/mvp.md`) was completed 2026-07-22 and deleted; its durable content lives in `architecture.md`.
