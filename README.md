# ODM

A CAD/3D modeling + animation framework built for LLM agents. The agent
writes JavaScript "doohickey" files; a long-running engine builds them into
solid geometry (Manifold CSG), shows the result live, and answers structured
queries over a CLI designed for agent feedback loops.

## Try it

```sh
cargo build --release
target/release/odm-engine examples/piston --headless &
cd examples/piston
../../target/release/odm render --t 1.0     # writes a PNG, prints its path
../../target/release/odm tree
```

Edit any `.js` file and re-run a command — every CLI call syncs and rebuilds
what changed.

## Layout

- `crates/odm-ir` — IR types + content hashing (blake3, canonical encoding)
- `crates/odm-store` — content-addressed store, generations, memo cache
- `crates/odm-kernel` — Manifold wrapper (CSG, extrude/revolve, raycast, cancellation)
- `crates/odm-js` — deno_core runtime: isolate-per-doohickey from a snapshot,
  framework API bindings, determinism freezing
- `crates/odm-build` — scheduler: generations, memoized demand-driven builds,
  in-flight dedup, cycle detection, cancellation
- `crates/odm-render` — wgpu renderer (offscreen PNG; viewer shares the code path)
- `crates/odm-engine` — the engine binary (socket server, viewer)
- `crates/odm-cli` — the `odm` CLI binary
- `framework/` — JS framework + vendored three.js subset (r185)
- `examples/` — example projects (double as integration tests)
- `docs/agent/` — docs for agents *using* ODM on a project

Design notes and decisions live in `notes/`; the MVP plan in `plans/mvp.md`.

## Notes

- First build clones the Manifold C++ sources (network needed once); see
  `issues/hermetic-manifold-build.md`.
- Determinism: builds are bit-reproducible within a process and across
  engines on the same platform (frozen Date, seeded Math.random, always-on
  Manifold determinism, content-addressed everything).
