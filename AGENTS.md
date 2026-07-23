## Overview
ODM is a CAD/3D modeling/animation framework and toolset for LLM agents. The agent writes JavaScript code to construct 3D models, and uses tools to inspect them. A long-running engine/viewer program allows the user to interact with the project and give the agent instructions. This repo contains the *implementation* of ODM, meaning it has the tools, prompts etc needed to make this all work.

## Workflow
- ODM is itself agent-built. You own the code.
- Refactor freely as needed. don't trust that existing code/comments/notes are necessarily correct, or existing design decisions are optimal.
- Only commit or push when explicitly asked. Git push may hang without user approval.
- Do not run formatting tools like `cargo fmt` unless explicitly asked.
- Keep prose, comments, errors, and commit messages short unless extra detail is genuinely useful.

## Running the viewer
Never run `odm-engine` without `--headless` outside a headless Wayland compositor —
it would open a window on the user's desktop. Unless the user explicitly asks for
that, use `--headless` (enough for almost everything: `odm render`, `build`, `tree`,
`inspect`, `raycast`) or `scripts/ui-shot.sh`, which supplies its own compositor.

When you need to *see* the UI (egui chrome, theme, layout — things `odm render`
never touches):

    cargo build --bins
    scripts/ui-shot.sh -o /tmp/shot.png examples/hello-bracket   # then Read the png

It starts `labwc` on wlroots' headless backend in a private `XDG_RUNTIME_DIR`
(`mktemp -d`, so concurrent agents never collide), launches the engine on it,
waits for the first built frame to settle, captures with `grim`, and tears down
compositor, engine and runtime dir on exit — including after SIGINT/SIGTERM. A
run killed with SIGKILL is reaped by the next run.

- `-s WxH` output size, `-d SECS` extra settle delay, `-k KEYS` send keystrokes
  via `wtype` before the shot (only `F` = frame is bound today).
- `scripts/ui-shot.sh --session CMD...` runs anything inside the compositor with
  `WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR` set, for multi-shot or interactive work.
  Pipe stdout through `stdbuf -oL` if you background a client and kill it later,
  or you lose its buffered output. Don't read stdin in `CMD`.
- No pointer injection is available (no ydotool/wlr virtual-pointer CLI), so
  mouse-driven UI can't be exercised — keyboard only.
- The engine socket path is fixed per project, so two runs on the same project
  dir share it; the second viewer renders fine but `odm` talks to the first
  engine. The script warns when this happens. Use distinct projects in parallel.

## Notes
The `notes/` directory contains your persistent notes about the project state. Create/edit/rename/split/delete notes as needed (without being asked) to keep them correct and maximally useful to you. Keep `notes/README.md` up to date with an index of what is where.

## Issues
Issues live in `issues/`. Do not solve them unless asked or the fix falls out of current work. Create/update issues for nontrivial problems discovered during other work. Delete confirmed-solved issues (move still-useful context into notes first).

## Plans
Future plans live in `plans/`. Do not execute them unless asked, or write new plans unless asked. Like issues, delete them and integrate their contents into your notes when they are complete.

## High-level Architecture
The core code is implemented in simple, safe Rust. Examples and projects using ODM are written in JavaScript.

### Framework
The ODM framework consists of JavaScript APIs useful for building and interacting with 3D objects. Low-level types (Vector3, Matrix4, etc) come from a vendored subset of Three.js (only what the framework actually uses); higher-level types (scenes, objects) are ODM's own API. Geometry lives engine-side, content-addressed; JS holds opaque handles, and vertex data crosses the boundary only on explicit request. Animation is `build(t)`: time is an ordinary context value, made cheap by memoization (dependency-tracked context reads; content-addressed outputs). An ODM project consists of composable pieces, called doohickeys, each implemented in a single js file. A doohickey can be thought of somewhat like React component. The main function, `build()`, is conceptually a pure function that maps arbitrary global and local context values to output, such as 3D or 2D geometry. `build()` may call API functions to interact with its arguments (eg checking an intersection ray with a 3D object it is given), but should never have access to state outside of its intended arguments (allowing partial invalidation and re-building of the project). There exists an API for invoking the `build()` of another doohickey and getting its output, although it is actually run in a different JavaScript context and may be memoized.

### Engine
The engine is a long-lived process. One engine runs per active project. It opens up a graphical viewer by default, but can also be run headless. V8 is embedded via deno_core, and each doohickey runs in its own isolate (created from a snapshot with the framework preloaded; Date/Math.random frozen for determinism). The geometry kernel is Manifold (mesh CSG). The viewer is egui + a custom wgpu renderer; viewer and headless/agent renders share one code path (pixel-identical). The engine loads and calls doohickey code, converts between JS and Rust types, implements framework APIs, implements the server side of the CLI socket, and renders the result. Builds are memoized against a content-addressed store and run on a background build thread (one build in flight, latest-wins); the UI never blocks on builds and always shows the last fully-built generation. Hot reload: every CLI query first syncs (content-hashes source files → a new generation), rebuilds what changed, then answers; inotify only triggers early syncs. Invariant: every published result is byte-equivalent to a from-scratch build of the current generation (consistency over avoiding redundant work). The engine never writes to ODM project files.

### CLI
The CLI is designed primarily to be used by agents working on an ODM project. It connects to a running engine over a socket. It can be used to create renders with various options, query built doohickeys, etc.

### Agent
An LLM coding agent (such as Claude Code, Codex, etc) works on an ODM project by editing doohickey code and using the CLI. The agent is run by the user independently, and there are not hard requirements on exactly what agent is used or how it works.
