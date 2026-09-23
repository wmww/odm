# Plan: Windows support (x86_64-pc-windows-msvc)

Written 2026-09-22. Order: after `transport.md` and `ci.md`; before
`macos.md` (the process-lifetime module and the platform-agnostic test
changes made here are what macOS inherits). Done means: a `windows` lane
in `test.yml` runs `cargo test --workspace` green on `windows-latest`,
and the viewer opens on a real Windows desktop.

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
     rejects them. Emit only when `CARGO_CFG_TARGET_OS` is linux/macos
     (build scripts see the *target* cfg via env, not `cfg!`). On Windows
     the dev binary finds `odm_dylib.dll` because cargo puts
     `target/debug/deps` on PATH for `cargo run`/`cargo test`; running
     `target\debug\odm.exe` by hand needs that dir on PATH — document,
     don't fix.
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
   - The `real-adapters` opt-in job (`ci.md`) is the end-to-end check on
     the lane.
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
7. **Viewer.** Add a hidden `ODM_VIEWER_SMOKE=1` mode: `odm run` (viewer)
   paints one frame, prints the adapter and window backend, exits 0. e2e
   test `viewer_paints_a_frame`, skipped only when no display is
   available (Linux CI has none; Windows and macOS runners have a desktop
   session). Also check by eye once, through `run.yml` uploading a
   screenshot: PowerShell `System.Drawing` capture of the primary screen
   after launching the viewer with a 5 s sleep — crude, but it is the one
   time a human looks at Windows chrome before release.
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

1. Add the `windows` lane to `run.yml`; self-test with `cargo --version`.
2. Items 1–2 until `cargo build --workspace` is green.
3. Items 3–6, 8; `cargo test --workspace` green via `run.yml`.
4. Item 7; one screenshot.
5. Add `windows` to `test.yml`'s matrix and lane choices.
6. Notes: architecture.md (process module, launcher, dirs, the
   case-check), build-environment.md (Windows lane facts, timings).
   Delete this plan.
