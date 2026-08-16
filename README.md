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
../../target/release/odm render '{"inputs": {"t": 1.0}}'   # writes a PNG, prints its path
../../target/release/odm inspect
```

Edit any `.js` file and re-run a command — every CLI call syncs and rebuilds
what changed.

`scripts/install.sh` builds and drops the binary in `~/.local/bin` (override
with `BINDIR=`); it is self-contained, so that is the whole install.

An agent gets the instructions ODM ships with from the project's own
`AGENTS.md`/`CLAUDE.md`: a new project is created with them, and every project
open re-syncs whatever sits between the `STANDARD ODM PROMPT` markers, so they
never go stale. An existing project without them is asked about once per open.
`odm docs prompt` prints the same text for pasting anywhere else.

The text is `docs/prompts/` — how to write doohickeys, and how to use the CLI
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
- `crates/odm-prompt` — the agent prompt text + the `AGENTS.md`/`CLAUDE.md`
  marked block the engine keeps current
- `crates/odm` — the one `odm` binary: `run` is the engine, the rest is the client
- `framework/` — JS framework + vendored three.js subset (r185)
- `examples/` — example projects (double as integration tests)
- `docs/` — for agents *using* ODM on a project: `docs/prompts/` is the
  short in-context layer (`odm docs prompt` prints it, compiled into
  the binary); `docs/api/` is the full JS API reference

Design notes and decisions live in `notes/`; known issues in `issues/`.

## Notes

- First build clones the Manifold C++ sources (network needed once); see
  `issues/hermetic-manifold-build.md`.
- Determinism: builds are bit-reproducible within a process and across
  engines on the same platform (frozen Date, seeded Math.random, always-on
  Manifold determinism, content-addressed everything).
