# Plan: macOS support (aarch64-apple-darwin)

Written 2026-09-22. Order: after `transport.md` and `ci.md`; **in
parallel with `windows.md`** (two agents, own worktrees; see "Parallel
with windows.md" at the end); before `release.md`. Done means: the
`macos` lane `ci.md` already declared in `test.yml` runs `cargo test
--workspace` green on `macos-15`, and the viewer opens on a Mac desktop.
Intel Macs are out of scope (release.md, "later").

No Mac here, and a macOS VM on this machine is both against Apple's
license and Metal-less, so the loop is `run.yml` with `lane=macos`, as
for Windows. macOS is the shorter port: the Unix code paths all apply
and the transport is already portable. To balance the pair, this agent
also owns two platform-agnostic pieces the Windows port needs: the
per-OS rpath in `crates/odm/build.rs` and the viewer smoke mode, both of
which can be built and tested on the Linux dev box.

## Lane

`runs-on: macos-15` (Apple silicon). Preinstalled: Xcode command-line
tools (clang, ld64), cmake, rustup, node, git. GPU: the runner VM exposes
Apple's paravirtual GPU, which supports Metal, so wgpu should find a
hardware-class adapter — confirm from the render tests' adapter line on
the first run. If it does not, there is no software Metal; the lane would
have to run with `ODM_TEST_NO_GPU=1` and lose the render tests, which is
worth knowing before anything else. Cache as the other lanes. Runner
minutes are free (public repo), so iterate freely.

## Port items, each with its test

1. **Build green.** Expected blockers:
   - `crates/odm/build.rs` (**this agent owns it, for all OSes**): rpath
     syntax is the same (`-Wl,-rpath,…`) but the token is `@loader_path`,
     not `$ORIGIN`; the toolchain libdir entry (for `libstd.dylib`) is
     the same idea. Emit per `CARGO_CFG_TARGET_OS`: `$ORIGIN` on linux,
     `@loader_path` on macos, nothing on windows (MSVC's linker rejects
     the flag — the Windows agent is waiting on this). Do it first and
     merge-request it early if the Windows agent is blocked; it is a
     ten-line change.
   - `odm-dylib` builds as `libodm_dylib.dylib`; check the dev binary
     runs standalone from `target/debug/` as it does on Linux.
   - rusty_v8 ships `aarch64-apple-darwin` prebuilts; Manifold's cmake
     builds with Apple clang (the sys crate has an Apple branch).
   - Anything else the log names.
2. **Agent process lifetime.** Process groups and `kill(-pgid)` work
   unchanged. There is no `PR_SET_PDEATHSIG`; the adapters exit on stdin
   EOF, which an engine death causes, and the process group covers their
   children on a clean shutdown. Accept that a *killed* engine on macOS
   relies on EOF. The test that pins the EOF contract (fake agent exits
   within the grace period when the client drops stdin without a
   shutdown) lives in `odm-agent/tests/client.rs`, which the **Windows
   agent owns** and writes as part of the process-module work; this agent
   verifies it on the lane after the merge.
3. **User directories.** Keep the XDG layout (`~/.config/odm`,
   `~/.local/share/odm`) on macOS: many CLI tools do, the config is
   hand-edited TOML rather than an app's preferences, and it keeps one
   Unix code path. Document in the README's install section.
4. **Filesystem.** APFS is case-insensitive by default: the exact-name
   import check is the Windows agent's (`windows.md` item 5); verify on
   the lane after the merge. The watcher is FSEvents-backed via notify;
   latency differs from inotify (events are coalesced). The `state.rs`
   watcher test's debounce assumptions may need the timeout scaling from
   `ci.md`; verify on the lane.
5. **Renders.** Metal. Analytic asserts pass; the e2e same-bytes render
   check holds on one machine. The `force_fallback_adapter` retry from
   the Windows plan does nothing useful here (no software Metal) but is
   harmless.
6. **Viewer** (**this agent owns the smoke mode, for all OSes**). Add a
   hidden `ODM_VIEWER_SMOKE=1` mode: `odm run` (viewer) paints one frame,
   prints the adapter and window backend to stdout, exits 0. Test in a
   new file `crates/odm/tests/viewer_smoke.rs` (not e2e.rs, which the
   Windows agent owns; the odm test crate links nothing heavy, so a
   second file is cheap): skipped only when no display is available
   (Linux CI has none; Windows and macOS runners have a desktop session).
   Build and verify it on the dev box under the gui-testing skill's
   session first. Then on the lane: `run.yml` + `screencapture -x
   out.png` uploaded as an artifact, checked by eye once.
   Known things to look at: the `SlowIdle` wrapper in `viewer/idle.rs`
   was written for Wayland's missing visibility signal and forces the
   loop to exit on close — confirm close-to-quit behaves and nothing spins
   with the window hidden (Cmd-H). Menu bar: egui draws its own, so no
   native menu; acceptable. The binary runs without an `.app` bundle
   (generic Dock icon).
7. **Symlinked agent files.** `CLAUDE.md → AGENTS.md` works as on Linux;
   the existing tests cover it.
8. **Transport on macOS.** `GenericNamespaced` maps to `/tmp/<name>`;
   the transport plan accepts that. The e2e suite's tempdir moves to
   `$TMPDIR` (a long per-user path on macOS), which is exactly what the
   path-length-free transport allows — a good check that it really is.

## Not in this plan

Ad-hoc signing (`codesign -s -` after strip), the quarantine README note,
the Homebrew tap, Intel builds: `release.md`.

## Steps

1. `crates/odm/build.rs` per-OS rpath (dev box: Linux still runs
   standalone from `target/debug/`); commit early.
2. `run.yml -f lane=macos` (the lane exists since `ci.md`): `cargo build
   --workspace 2>&1`, then `cargo test --test render` for the adapter
   line. Item 1's remaining blockers until the build is green; items 3,
   5, 8 until `cargo test --workspace` is green (items 2 and 4 are
   verified after the Windows merge).
3. Item 6 on the dev box, then on the lane; one screenshot; fix what it
   shows.
4. Rebase on the Windows merge; verify items 2 and 4 and the smoke test
   on both lanes.
5. Notes: the **macOS** stub under "Platforms" in architecture.md (dirs,
   EOF contract, viewer quirks, rpath) and the **macOS lane** stub in
   build-environment.md (facts, timings). Delete this plan.

## Parallel with windows.md

Two agents, two worktrees; the Windows change merges first, this one
rebases. Ownership:

- **This agent edits:** `crates/odm/build.rs`, `crates/odm-engine/src/
  viewer/{mod,idle}.rs` and `crates/odm-engine/src/lib.rs` (the smoke
  mode's entry), `crates/odm/tests/viewer_smoke.rs` (new), and the macOS
  stubs in both notes.
- **This agent does not edit:** anything under `crates/odm-agent`,
  `crates/odm-config`, `crates/odm-build`, `crates/odm-prompt`,
  `crates/odm-render`, `crates/odm-engine/src/{agent,watcher.rs}`,
  `crates/odm/tests/e2e.rs`, workflow files, `Cargo.lock` (no new
  dependencies here), the Windows stubs in the notes.
- If the macOS build turns out to need a change in a Windows-owned file,
  do the minimum in the worktree uncommitted, and hand the change to the
  Windows agent (or wait for its merge) rather than committing it.
