# ODM

A CAD/3D modeling + animation framework built for LLM agents. The agent
writes JavaScript "doohickey" files; a long-running engine builds them into
solid geometry (Manifold CSG), shows the result live, and answers structured
queries over a CLI designed for agent feedback loops.

## Try it

```sh
cargo build --release
target/release/odm run examples/piston --headless &   # drop --headless for the viewer
cd examples/piston
../../target/release/odm render --t 1.0     # writes a PNG, prints its path
../../target/release/odm tree
```

Edit any `.js` file and re-run a command — every CLI call syncs and rebuilds
what changed.

To point an agent at a project, give it the instructions ODM ships with:

```sh
odm prompt > AGENTS.md     # or CLAUDE.md, or paste into a system prompt
```

Those are `prompts/` — how to write doohickeys, and how to use the CLI
(including `odm poll` / `odm say`, which carry messages between the user's
viewer and the agent).

## Layout

- `crates/odm-ir` — IR types + content hashing (blake3, canonical encoding)
- `crates/odm-store` — content-addressed store, generations, memo cache
- `crates/odm-kernel` — Manifold wrapper (CSG, extrude/revolve, raycast, cancellation)
- `crates/odm-js` — deno_core runtime: isolate-per-doohickey from a snapshot,
  framework API bindings, determinism freezing
- `crates/odm-build` — scheduler: generations, memoized demand-driven builds,
  in-flight dedup, cycle detection, cancellation
- `crates/odm-render` — wgpu renderer (offscreen PNG; viewer shares the code path)
- `crates/odm-engine` — the engine (socket server, viewer)
- `crates/odm-cli` — the client commands (JSON over the project socket)
- `crates/odm` — the one `odm` binary: `run` is the engine, the rest is the client
- `framework/` — JS framework + vendored three.js subset (r185)
- `examples/` — example projects (double as integration tests)
- `prompts/` — the instructions for agents *using* ODM on a project;
  `odm prompt` prints them, compiled into the binary

Design notes and decisions live in `notes/`; known issues in `issues/`.

## Notes

- First build clones the Manifold C++ sources (network needed once); see
  `issues/hermetic-manifold-build.md`.
- Determinism: builds are bit-reproducible within a process and across
  engines on the same platform (frozen Date, seeded Math.random, always-on
  Manifold determinism, content-addressed everything).
