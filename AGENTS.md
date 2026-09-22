## Overview
ODM is a CAD/3D modeling/animation framework and toolset for LLM agents. The agent writes JavaScript code to construct 3D models, and uses tools to inspect them. A long-running engine/viewer program allows the user to interact with the project and give the agent instructions. This repo contains the *implementation* of ODM, meaning it has the tools, prompts etc needed to make this all work.

## Notes
The `notes/` directory contains your persistent notes about the project state. Create/edit/rename/split/delete notes as needed (without being asked) to keep them correct and maximally useful to you. Keep notes concise, remove parts or whole notes that are unimportant or obvious. Keep `notes/README.md` up to date with an index of what is where.

## Issues
Issues live in `issues/`. Do not solve them unless asked or the fix falls out of current work. Create/update issues for nontrivial problems discovered during other work. Delete confirmed-solved issues (move still-useful context into notes first).

## Plans
Future plans live in `plans/`. Do not execute them unless asked, or write new plans unless asked. Like issues, delete them and integrate their contents into your notes when they are complete.

## Workflow
- This project is agent-built, you own the code.
- Refactor freely as needed. don't trust that existing code/comments/notes are necessarily correct, or existing design decisions are optimal.
- Unless otherwise asked, commit when you've completed your task.
- Only pull/push when explicitly asked. Git push may hang without user approval.
- Commit to the current branch unless asked, don't make feature branches.
- Do not run code formatting tools unless explicitly asked.
- Keep prose, comments, errors, and commit messages short unless extra detail is genuinely useful.
- Build with `cargo build/test --workspace`, not `-p` subsets, and never set `CARGO_INCREMENTAL`: inconsistent invocations mint a whole new set of half-GB artifact hashes (see notes/build-environment.md). If `target/` balloons anyway, run `scripts/sweep-target.py`.
- Avoid opening windows in the user's desktop, to test, interact with and screenshot GUI apps use the gui-testing skill from https://github.com/wmww/agent-skills.

## Running the engine
Use `odm run --headless` by default. This will allow you to interact with the engine and take 3D renders with the CLI. Only run the viewer if you need to screenshot/test the UI, and use the gui-testing skill when you do.

## High-level Architecture
The core code is implemented in simple, safe Rust. Examples and projects using ODM are written in JavaScript.

### Framework
The ODM framework consists of JavaScript APIs useful for building and interacting with 3D objects. Low-level types (Vector3, Matrix4, etc) come from a vendored subset of Three.js (only what the framework actually uses); higher-level types (scenes, objects) are ODM's own API. Geometry lives engine-side, content-addressed; JS holds opaque handles, and vertex data crosses the boundary only on explicit request. An ODM project consists of composable pieces, called parts, each implemented in a single js file. A part can be thought of somewhat like a React component. It declares its inputs (`export const meta`) as profiled JSON Schemas; plain inputs come from the immediate caller, cascade inputs resolve up the invoke chain (view outermost). The main function, `build()`, is conceptually a pure function that maps those inputs to output, such as 3D or 2D geometry. Animation is just a ranged cascade input `t` — time is an ordinary input, made cheap by memoization (dependency-tracked input reads; content-addressed outputs). `build()` may call API functions to interact with its arguments (eg checking an intersection ray with a 3D object it is given), but should never have access to state outside of its intended arguments (allowing partial invalidation and re-building of the project). There exists an API for invoking the `build()` of another part and getting its output, although it is actually run in a different JavaScript context and may be memoized.

### Engine
The engine is a long-lived process. One engine runs per active project. It opens up a graphical viewer by default, but can also be run headless. V8 is embedded via deno_core, and each part runs in its own isolate (created from a snapshot with the framework preloaded; Date/Math.random frozen for determinism). The geometry kernel is Manifold (mesh CSG). The viewer is egui + a custom wgpu renderer; viewer and headless/agent renders share one code path (pixel-identical). The engine loads and calls part code, converts between JS and Rust types, implements framework APIs, implements the server side of the CLI socket, and renders the result. Everything is evaluated as a *view* — (part path, args, cascade values) — with viewer tabs and CLI queries each holding one; `root.js` is only the default-target convention. Builds are memoized against a content-addressed store and run on a background build thread (one build in flight, per-view latest-wins); the UI never blocks on builds and always shows each tab's last fully-built result. Hot reload: every CLI query first syncs (content-hashes source files → a new generation), rebuilds what changed, then answers; inotify only triggers early syncs. Invariant: every published result is byte-equivalent to a from-scratch build of the current generation (consistency over avoiding redundant work). The engine never writes to an existing project's files, with two exceptions: the `engine` version value in `odm.toml`, recorded when a project is opened; and the agent files (`AGENTS.md`/`CLAUDE.md`), where the standard prompt is kept current inside its marker pair — a marked block is the file's opt-in, and anything else needs the user's yes in a viewer question box. (File ▸ New Project authors a new project's first files.)

### CLI
The CLI is designed primarily to be used by agents working on an ODM project. It connects to a running engine over a socket. It can be used to create renders with various options, query built parts, etc.

### Agent
An LLM coding agent (such as Claude Code, Codex, etc) works on an ODM project by editing part code and using the CLI. The viewer runs the agent the user picked as a child process and talks to it over ACP (Agent Client Protocol) — the Agent panel is the conversation; the CLI stays the agent's only tool surface into ODM. Headless never spawns one.
