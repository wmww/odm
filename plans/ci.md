# Plan: CI on GitHub Actions (Linux x86_64 + aarch64)

Written 2026-09-22. Order: `transport.md` → **this** → `windows.md` →
`macos.md` → `release.md`. This plan is independent of the transport work
(nothing here is Unix-specific) but the later plans build on both.

## Decisions (user, 2026-09-22)

- GitHub Actions, **manual trigger only**: every workflow is
  `on: workflow_dispatch`. Nothing runs on push or PR.
- Lanes now: Linux x86_64 and Linux aarch64, both in the same Ubuntu 22.04
  container that release builds will use (glibc 2.35 floor). Windows and
  macOS lanes come from their own plans, into the same workflows.
- No LLM API keys on runners, ever. Agent coverage is the fake ACP agent
  (already the suite's approach), plus an opt-in real-adapter smoke that
  needs no key (below).

## Pieces

### `rust-toolchain.toml`

Pin the toolchain (`1.93.0`, the dev box's) so runners and the dev box
compile the same thing. Today nothing pins it.

### `scripts/Containerfile`

`FROM ubuntu:22.04`, one file for both architectures (only the rust target
differs, and rustup picks it from `uname`). Installs: rustup + the pinned
toolchain (+ `wasm32-unknown-unknown`), build-essential, cmake, ninja,
clang, lld (for `wasm-ld`), libc++-dev (the wasm shim wants its headers —
see `notes/web-export.md`; on 22.04 that is `libc++-14-dev`), git, python3,
pkg-config, node 22 (nodesource; `node_bundle.rs` needs it),
wasm-bindgen-cli at the version xtask expects, and `mesa-vulkan-drivers` +
`libvulkan1` for lavapipe (the render tests fail loudly without an
adapter, which is the behaviour we want: never `ODM_TEST_NO_GPU` on a
Linux lane). Also the X11/Wayland dev headers eframe's build needs
(`libxkbcommon-dev`, `libwayland-dev`, `libx11-dev`…): check with a first
build, add what fails.

Build and publish with a `container.yml` workflow (manual): buildx for
`linux/amd64,linux/arm64`, push to `ghcr.io/<owner>/odm-build:<sha of
Containerfile>` and `:latest`. The test workflow references the tag. The
same file builds locally with `podman build` for reproducing a CI failure
on the dev box — write that invocation into `notes/build-environment.md`.

### `.github/workflows/test.yml`

`workflow_dispatch` inputs: `ref` (branch/sha, default the branch the
workflow is run from) and `lanes` (choice: `all`, `linux-x86_64`,
`linux-arm64`; Windows/macOS plans add theirs). Matrix:

| lane          | runs-on            | container            |
|---------------|--------------------|----------------------|
| linux-x86_64  | `ubuntu-latest`    | `ghcr.io/…/odm-build`|
| linux-arm64   | `ubuntu-24.04-arm` | same image, arm64    |

Steps: checkout `ref`; restore cache (`~/.cargo/registry`, `~/.cargo/git`,
`target/`) keyed on lane + `Cargo.lock` + `rust-toolchain.toml` +
Containerfile tag; `cargo test --workspace` — **one invocation shape**, the
same one the notes mandate locally, so the cache never grows a second
artifact universe (`notes/build-environment.md`); save cache on success or
failure (a failed test run still warms the next one); on failure upload
`target/ci-out/` if present (tests that want to leave evidence write
there — nothing does yet).

V8 is never compiled: rusty_v8 downloads a ~180 MiB prebuilt into
`target/build/v8-*`, and a restored cache keeps it (its rerun triggers are
cached registry files and env vars that are constant on a runner), so the
download happens once per cache miss. `RUSTY_V8_ARCHIVE` could point at a
copy baked into the image; not worth it at GitHub's network speed.

**Cache budget:** GitHub caps a repo's caches at 10 GB total, LRU. Four
lanes each saving a 5–8 GB target dir would evict each other and every
run would be cold. Keep each lane's saved cache to ~2 GB: before saving,
delete `target/*/incremental`, the workspace crates' own artifacts (they
rebuild in seconds, as `scripts/seed-target.sh` already relies on) and
uplifted binaries; if that is not enough, `CARGO_PROFILE_DEV_DEBUG=0` in
CI, or cache only `~/.cargo` plus `target/build/{v8,manifold-csg-sys}-*`
and accept recompiling the ~500 registry crates (~5–8 min).

Disk: `ubuntu-latest` has ~20 GB free. A dev-profile target with
line-tables and the dylib is ~5–8 GB; if the cache save trips the limit,
set `CARGO_PROFILE_DEV_DEBUG=0` for CI only (its own cache key, so no
universe churn on the dev box).

Expected timings (guesses, measure on first run): cold ~20 min (V8
prebuilt download 180 MiB, Manifold clone + cmake, ~500 crates), warm
~3–5 min. If the arm64 runner is much slower, make the lane input default
to x86_64 only.

### `.github/workflows/run.yml` — the debugging lane

The tool the Windows and macOS plans lean on, defined here so the Linux
lanes get it first: `workflow_dispatch` with inputs `lane`, `ref`, and
`command` (a shell line). Runs the command in the lane's environment
(inside the container on Linux), captures stdout+stderr to a log, uploads
the log and `target/ci-out/` as artifacts, and exits with the command's
status. Driving it from the dev box:

```
gh workflow run run.yml -f lane=linux-arm64 -f ref=$(git rev-parse HEAD) \
  -f command='cargo test --workspace --test e2e 2>&1 | tail -100'
gh run watch $(gh run list --workflow=run.yml -L1 --json databaseId -q '.[0].databaseId')
gh run view --log-failed
```

Commit-then-run is the loop; a cache-warm iteration should be ~5 min.
Optional for interactive sessions: an `ssh` input that starts
`mxschmitt/action-tmate` after the command (public relay; only when asked).

### Agent adapters without keys

- The gate already covers the ACP client end-to-end against
  `odm-fake-agent` (permissions, cancel, crash, stderr floods, replay,
  logged-out). That is what runs on every lane.
- **Opt-in `real-adapters` job** (`test.yml` input `real_adapters=true`;
  needs network + npm, ~600 MiB): `npm install` the pinned
  `claude-agent-acp` and `codex-acp` into a temp data dir exactly as
  `agent::table::install` does, spawn each through the engine's own
  launcher with no credentials, and assert the *logged-out* path: the
  adapter starts, speaks ACP (`initialize` succeeds), the first prompt
  ends in the auth-required outcome the host already handles, then
  shutdown kills the process group and nothing is left running. This
  exercises install, spawn, PATH/`.cmd` resolution, and group kill with
  the real binaries and zero API calls — exactly the surface the ports put
  at risk. New test file: `crates/odm-engine/tests/real_adapters.rs`,
  `#[ignore]` by default, run with `--ignored` by the job.
- Mocking the Anthropic/OpenAI APIs behind a base-URL override to get a
  whole turn through a real adapter is possible but couples us to each
  adapter's internals; not planned.

## Tests to add or change for CI

- **Deadlines.** Runner cores are slow and shared. Audit every fixed wait
  in tests (`Engine::start` 10 s in e2e, `recv_timeout(10 s)` in
  odm-agent, `wait_until` in engine agent tests, the fake agent's pauses)
  and scale them by `ODM_TEST_TIMEOUT_SCALE` (default 1; the workflow sets
  4). A deadline that only ever fires on a slow runner is noise, not a
  finding.
- **aarch64 first run.** Nothing has run on arm64. Expect: V8 prebuilt for
  `aarch64-unknown-linux-gnu` (rusty_v8 ships it), Manifold builds from
  source, lavapipe on arm64. The analytic render asserts and the kernel
  precision tests have tolerances; if any pin bit-exact float results,
  loosen to a tolerance rather than fork goldens.
- **Adapter announcement.** The render tests print the adapter once; the
  workflow greps the log for `llvmpipe` on Linux lanes and fails if it is
  absent — a silent switch to no-GPU skipping would otherwise pass green.
- **`run.yml` self-test:** run it once with `cargo --version` per lane
  before relying on it.

## Docs and notes

- `notes/architecture.md` Testing: replace "There is no CI and none is
  planned" with the manual-trigger workflows and the `run.yml` loop.
- `notes/build-environment.md`: the container recipe, how to reproduce a
  lane locally with podman, cache-key rules.
- README developer section (or `DEVELOPING.md` when release.md moves it):
  how to trigger a lane with `gh`.
- Remove this plan when the two Linux lanes are green and `run.yml` works;
  keep the lane table and the loop in architecture.md.

## Steps

1. `rust-toolchain.toml`; Containerfile; build it locally with podman and
   get `cargo test --workspace` green inside it on the dev box (this is
   also the first time the suite runs against glibc 2.35 and lavapipe).
2. `container.yml`, push the image to GHCR (repo is private today: GHCR
   packages inherit repo visibility, and the workflow's `GITHUB_TOKEN` can
   pull it).
3. `test.yml` with the two Linux lanes; timeout scaling; adapter grep.
4. `run.yml`; self-test.
5. `real_adapters.rs` and the opt-in job.
6. Notes and docs; commit; delete this plan.
