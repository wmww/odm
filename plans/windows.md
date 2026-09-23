# Plan: Windows support (x86_64-pc-windows-msvc)

Written 2026-09-22. Order: after `transport.md` and `ci.md`; **in
parallel with `macos.md`** (two agents, own worktrees; see "Parallel with
macos.md" at the end — this agent owns nearly all the shared,
platform-agnostic changes, the macOS agent owns the rpath and the viewer
smoke mode). Done means: the `windows` lane `ci.md` already declared in
`test.yml` runs `cargo test --workspace` green on `windows-latest`, and
the viewer opens on a real Windows desktop.

There is no Windows machine. Every iteration goes through `run.yml`
(`ci.md`): commit, `gh workflow run run.yml -f lane=windows -f
command=…`, read the log. Budget ~5 min per warm iteration, 25+ min cold,
so batch fixes: get a full `cargo build --workspace 2>&1` log, fix
everything it names, repeat.

## Lane

`runs-on: windows-latest`, no container. Preinstalled: MSVC Build Tools,
cmake, ninja, rustup (`rust-toolchain.toml` picks the version), node 22,
git. Nothing to install. No GPU → wgpu on DX12 finds only WARP
(Microsoft's software rasterizer), which needs the fallback in "Renders"
below. Cache as the Linux lanes, keyed per lane.

## Port items, each with its test

1. **Build green.** Known compile blockers, all found by grep, the rest
   by the first log:
   - `crates/odm-agent/src/process.rs` uses
     `std::os::unix::process::CommandExt` unconditionally → item 2.
   - `crates/odm/build.rs` emits `-Wl,-rpath` link args; MSVC's linker
     rejects them. **The macOS agent owns this file** (it needs
     `@loader_path` there) and makes it emit per `CARGO_CFG_TARGET_OS`:
     nothing on Windows. Until that merges, work around it locally in the
     worktree without committing the file. On Windows the dev binary
     finds `odm_dylib.dll` because cargo puts `target/debug/deps` on PATH
     for `cargo run`/`cargo test`; running `target\debug\odm.exe` by hand
     needs that dir on PATH — document, don't fix.
   - `crates/odm-prompt/src/fs.rs`: the `#[cfg(not(unix))]` branch
     exists; check what it does (a copy of AGENTS.md? nothing?) and make
     it a copy that the marker update keeps in step. Test: the existing
     symlink tests get a non-unix twin asserting both files carry the
     current marked block after an update.
   - Anything else the log names.
2. **Agent process lifetime** — `odm-agent/src/process/{mod,unix,windows}.rs`.
   Unix keeps process groups + `kill(-pgid)`, `prctl` stays Linux-only.
   Windows: a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
   created before the spawn, the child assigned right after `spawn()`
   returns (the window before assignment is microseconds and the adapter
   spawns no children that early); `kill_group` = `TerminateJobObject`;
   the job handle lives in the same struct as today's `pgid` and closing
   it on engine death kills everything — better than Linux's PDEATHSIG,
   which only reaches the direct child. Dependency: `windows-sys`
   (`Win32_System_JobObjects`, `Win32_Foundation`), Windows target only.
   Test (all platforms): a fake-agent step `{"spawn_child": ms}` starts a
   sleeping grandchild and reports its pid; after `shutdown_wait` the test
   asserts the grandchild is gone (`sysinfo`-free: on Unix `kill(pid, 0)`
   fails with ESRCH, on Windows `OpenProcess` fails or
   `GetExitCodeProcess` ≠ STILL_ACTIVE — the one small cfg in the test).
3. **Finding and launching agents** — `odm-engine/src/agent/table.rs`.
   - `on_path` must honour `PATHEXT` (`.exe`, `.cmd`, `.bat`): a
     `which`-style helper that returns the resolved full path with its
     extension, used both for the check and for what gets spawned.
     `Command::new("npm")` cannot find `npm.cmd`; std runs `.cmd`/`.bat`
     through `cmd.exe` with correct quoting *once given the full path*.
     Unit test with a temp PATH dir holding `tool.cmd`.
   - npm-installed adapters: stop launching `node_modules/.bin/<bin>` (a
     shell script on Unix, a `.cmd` shim on Windows) and instead read the
     package's `package.json` `bin` entry and launch
     `node <dir>/node_modules/<package>/<bin.js>`. One code path on every
     platform, no shims. Test: a fake package dir with a `bin` entry
     resolves to the node invocation.
   - `test.yml -f real_adapters=true` (`agent::real_adapters`, which
     calls `table::npm_command`) is the end-to-end check on the lane; its
     leftover-process check is Linux-only (`/proc`) — add a Windows one.
4. **User directories** — `odm-config`: `config_path()` → `%APPDATA%\odm\
   config.toml`, `data_dir()` → `%LOCALAPPDATA%\odm`; on Unix unchanged.
   `viewer/open.rs`'s `~` expansion reads `USERPROFILE` when `HOME` is
   absent. Hand-written (three env vars), no `dirs` crate. Unit tests
   drive both branches by env var, on every platform, so the Windows
   mapping is checked on Linux too.
5. **Paths and filesystems.**
   - Watcher: canonicalize the project root once and compare event paths
     canonicalized the same way (`\\?\` prefixes otherwise never match).
     The existing watcher test in `state.rs` covers it once the lane runs.
   - Case-insensitive filesystems: at sync time, when an import resolves,
     check the on-disk name matches the specifier exactly (read the parent
     dir once per file). Mismatch is a build error naming both spellings.
     Runs on every platform (cheap; deterministic), so a project made on
     Windows or macOS that would break on Linux fails where it is made.
     Test: `Foo.js` imported as `foo.js` is an error — on Linux the
     resolution itself fails, so the test asserts the error either way.
   - Line endings: the marker splice in `odm-prompt` must preserve a
     CRLF file's endings (git autocrlf makes these common on Windows).
     Test: splice into a CRLF file, assert no bare `\n` appears.
   - JSON responses carry `\`-separated paths on Windows. Fine as JSON;
     `docs/cli.md` examples stay `/`.
6. **Renders without a GPU.** `Renderer::new`: if `request_adapter` finds
   nothing, retry with `force_fallback_adapter: true` and say so in the
   adapter string. WARP on Windows, and lavapipe on Linux when a hardware
   adapter is absent, come up through this path. The render tests are
   analytic (no pixel goldens), so they pass on WARP. The e2e
   `render_writes_a_deterministic_png` asserts same-bytes across two runs
   on one machine, which holds on WARP.
7. **Viewer.** The `ODM_VIEWER_SMOKE=1` mode and its test
   (`crates/odm/tests/viewer_smoke.rs`) are **the macOS agent's**; once
   merged, run it on the Windows lane. What is this agent's: check by eye
   once, through `run.yml` uploading a screenshot — PowerShell
   `System.Drawing` capture of the primary screen after launching the
   viewer with a 5 s sleep — crude, but it is the one time a human looks
   at Windows chrome before release.
8. **Tests that are Unix-shaped.** `odm-agent/tests/client.rs`
   `the_agents_odm_is_ours` runs `sh -c`: make the fake agent able to
   report `PATH` and cwd (`{"print_env": ["PATH"]}` step) and drop the
   shell; the `PermissionsExt` chmod becomes cfg(unix). e2e's SIGKILL
   comments: `Child::kill` is `TerminateProcess` on Windows, same effect.
   `node_bundle.rs` runs `node`: fine. Anything with `/tmp`: gone with
   the transport plan.
9. **Feedback `platform()`** already gates the Linux detail; Windows
   reports `windows x86_64`. Fine.

## Not in this plan

Packaging (zip, `odm.exe`, PATH instructions, winget/scoop) and the
release lane: `release.md`. Codex's Windows sandbox vs named pipes: the
mailbox fallback covers it either way; note the outcome when the
real-adapters job first runs there.

## Steps

1. `run.yml -f lane=windows -f command='cargo build --workspace 2>&1'`
   (the lane exists since `ci.md`); read the whole log.
2. Items 1–2 until `cargo build --workspace` is green.
3. Items 3–6, 8; `cargo test --workspace` green via `run.yml`.
4. Item 7's screenshot; after the macOS merge, the smoke test on this
   lane.
5. Notes: the **Windows** stub under "Platforms" in architecture.md
   (process module, launcher, dirs, the case-check) and the **Windows
   lane** stub in build-environment.md (facts, timings). Delete this
   plan.

## Parallel with macos.md

Two agents, two worktrees; merge this one first (it is the larger
change), the macOS agent rebases. Ownership:

- **This agent edits:** `crates/odm-agent/src/process/**` (new split),
  `crates/odm-agent/src/bin/fake_agent.rs` (`spawn_child`, `print_env`
  steps), `crates/odm-agent/tests/client.rs` (grandchild-kill test, the
  EOF-exit test from macos.md item 2 — same file, same module, so it is
  written here), `crates/odm-engine/src/agent/table.rs`,
  `crates/odm-config/**`, `crates/odm-engine/src/viewer/open.rs`,
  `crates/odm-engine/src/watcher.rs`, `crates/odm-build/src/sources.rs`
  (case check), `crates/odm-prompt/src/fs.rs`, `crates/odm-render/src/
  gpu.rs`, `crates/odm/tests/e2e.rs`, workspace `Cargo.toml`/`Cargo.lock`
  (windows-sys), and the Windows stubs in both notes.
- **This agent does not edit:** `crates/odm/build.rs`,
  `crates/odm-engine/src/viewer/{mod,idle}.rs`, `crates/odm-engine/src/
  lib.rs` (viewer entry), `crates/odm/tests/viewer_smoke.rs`, workflow
  files, the macOS stubs in the notes.
- Everything this agent writes is platform-agnostic or cfg'd per OS with
  the Unix branch unchanged, so the macOS agent's tree keeps building
  throughout.
