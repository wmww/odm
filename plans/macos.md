# Plan: macOS support (aarch64-apple-darwin)

Written 2026-09-22. Order: after `transport.md`, `ci.md`, `windows.md`;
before `release.md`. Done means: a `macos` lane in `test.yml` runs
`cargo test --workspace` green on `macos-15`, and the viewer opens on a
Mac desktop. Intel Macs are out of scope (release.md, "later").

No Mac here, and a macOS VM on this machine is both against Apple's
license and Metal-less, so the loop is `run.yml` with `lane=macos`, as
for Windows. macOS should be the shorter port: the Unix code paths all
apply, the transport is already portable, and Windows will have made the
tests platform-agnostic.

## Lane

`runs-on: macos-15` (Apple silicon). Preinstalled: Xcode command-line
tools (clang, ld64), cmake, rustup, node, git. GPU: the runner VM exposes
Apple's paravirtual GPU, which supports Metal, so wgpu should find a
hardware-class adapter — confirm from the render tests' adapter line on
the first run. If it does not, there is no software Metal; the lane would
have to run with `ODM_TEST_NO_GPU=1` and lose the render tests, which is
worth knowing before anything else. Cache as the other lanes. Cost: free
once the repo is public; 10× Linux minutes on the free tier before, so
this plan can wait for the flip if minutes matter.

## Port items, each with its test

1. **Build green.** Expected blockers:
   - `crates/odm/build.rs`: rpath syntax is the same (`-Wl,-rpath,…`) but
     the token is `@loader_path`, not `$ORIGIN`; the toolchain libdir
     entry (for `libstd.dylib`) is the same idea. Emit per
     `CARGO_CFG_TARGET_OS`.
   - `odm-dylib` builds as `libodm_dylib.dylib`; check the dev binary
     runs standalone from `target/debug/` as it does on Linux.
   - rusty_v8 ships `aarch64-apple-darwin` prebuilts; Manifold's cmake
     builds with Apple clang (the sys crate has an Apple branch).
   - Anything else the log names.
2. **Agent process lifetime.** Process groups and `kill(-pgid)` work
   unchanged. There is no `PR_SET_PDEATHSIG`; the adapters exit on stdin
   EOF, which an engine death causes, and the process group covers their
   children on a clean shutdown. Accept that a *killed* engine on macOS
   relies on EOF. Test (all platforms): the fake agent, launched by the
   client, exits within the grace period when the client drops its stdin
   without a shutdown — pins the EOF contract the macOS story rests on.
3. **User directories.** Keep the XDG layout (`~/.config/odm`,
   `~/.local/share/odm`) on macOS: many CLI tools do, the config is
   hand-edited TOML rather than an app's preferences, and it keeps one
   Unix code path. Document in the README's install section.
4. **Filesystem.** APFS is case-insensitive by default: the exact-name
   import check from `windows.md` item 5 applies as-is. The watcher is
   FSEvents-backed via notify; latency differs from inotify (events are
   coalesced). The `state.rs` watcher test's debounce assumptions may need
   the timeout scaling from `ci.md`; verify on the lane.
5. **Renders.** Metal. Analytic asserts pass; the e2e same-bytes render
   check holds on one machine. The `force_fallback_adapter` retry from
   the Windows plan does nothing useful here (no software Metal) but is
   harmless.
6. **Viewer.** `ODM_VIEWER_SMOKE=1` from the Windows plan; e2e
   `viewer_paints_a_frame` runs (runner has a desktop). Check by eye once
   via `run.yml` + `screencapture -x out.png` uploaded as an artifact.
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

1. Add the `macos` lane to `run.yml`; self-test; read the adapter line
   from a `cargo test --test render` run.
2. Item 1 until the build is green; items 2–5 and 8 until
   `cargo test --workspace` is green.
3. Item 6; one screenshot; fix what it shows.
4. Add `macos` to `test.yml`'s matrix and lane choices.
5. Notes: architecture.md (macOS facts: dirs, EOF contract, viewer
   quirks), build-environment.md (lane facts, timings). Delete this plan.
