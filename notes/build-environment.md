# Build environment

Machine facts: 24 cores, Radeon GPU (no lavapipe installed), cmake 4.4 +
ninja, node 26.4, rustc 1.93
(edition 2024), network available. Clean Manifold (clone + cmake) build ~37s.

## Disk: where debug-build bytes actually go

Measured 2026-08-17, before the fixes below: `target/` hit 31 GiB and two
worktree targets hit ~13 GiB *unique* data each. Attribution:

- **Not V8.** The prebuilt `librusty_v8.a` contributes only ~31 MiB of code
  and ~16 MiB of debug lines per linked binary.
- **Full Rust debuginfo was ~85% of every binary**: `odm` was 630 MiB, of
  which ~525 MiB DWARF for the dep tree and 48 MiB actual `.text`. Fix:
  `[profile.dev] debug = "line-tables-only"` (nobody here runs a debugger;
  backtraces keep file:line) → `odm` is 245 MiB.
- **~30 binaries per full build**: `--all-targets` links a test binary per
  `tests/*.rs` file, each embedding the whole stack. odm-build's 8 files are
  now modules of one `tests/suite/` binary (see architecture.md, Testing).
- **cargo keeps every artifact generation forever**, and each distinct
  invocation shape mints a new one: toggling `CARGO_INCREMENTAL` changes the
  profile hash (verified empirically — one toggle = a full new set of
  workspace artifact filenames), and `-p` subset builds unify features
  differently than `--workspace` (resolver v3), cascading new `-C metadata`
  into every workspace crate above the affected dep. One agent session was
  observed minting ~7 such universes (10 GiB) in an afternoon. Mitigation:
  stick to `--workspace`, never set `CARGO_INCREMENTAL` (rule in AGENTS.md);
  `resolver.feature-unification = "workspace"` would pin the -p case but is
  still nightly-only in cargo 1.93. The sweep below self-heals any mess.

## Every checkout gets its own target dir, seeded from the main one

`scripts/seed-target.sh` gives a linked checkout its own `target/`, copied from
the main checkout's the first time it runs. Run by `.wt-hooks/create`, which the
worktree workflow executes in a new worktree right after creating it (`done` and
`wipe` hooks exist too; nothing needs them yet). Idempotent, and a no-op in the
main checkout, outside a git repo, or once `target/` exists — so run it by hand
in a worktree made outside that workflow.

Measured 2026-07-24: seed 0.8 s, first build **6.2 s** (the 8 local crates and
nothing else), 1.7 GiB of real disk once built — versus a ~3.6 GiB cold build
that re-downloads the 177 MiB prebuilt `librusty_v8.a` and re-runs the
Manifold/TBB cmake build. The main checkout's 13,075 target files were byte- and
stat-identical afterwards.

What the seed does, and why each part:

- `deps/` **hardlinked** (5.7 GiB, free). rustc *replaces* an artifact it
  rebuilds — unlink, then create — so a rebuild here never writes through to the
  peer. Verified: the peer's rlib kept its inode and content, link count just
  dropped back to 1.
- `.fingerprint/`, `build/`, `gn_out/` **copied** (~700 MiB). These cargo *does*
  rewrite in place, and `build/*/output` records absolute paths into the target
  dir it ran under, so the copies get those paths rewritten to point here.
- local-crate artifacts, `incremental/`, uplifted binaries **skipped**. This
  checkout builds its own in 6 s. The purge also drops every extensionless
  file in `deps/` — integration-test binaries are named after the test
  *file* (`report-<hash>`), so a by-crate-name purge misses them.

Do not "optimize" the copy into a `cp -al` of the whole tree. Measured: the
seeded checkout's build then rewrote the *peer's*
`deps/odm_render-<hash>.d` and
`.fingerprint/odm-render-<hash>/dep-lib-odm_render` through the hardlink, in
place, leaving main's dep-info pointing at the other checkout's target dir.

Trap worth remembering: rewriting the paths in `build/*/output` must preserve
the file's mtime (`touch -r`) and skip files that don't contain the old path.
Cargo compares a build script's output mtime against its consumers', so a
gratuitous rewrite marks every crate with a build script stale — that turned a
6 s build into 1m20 until it was fixed.

### Why not one shared dir

That's what this used to do (`shared-target.sh`, `build.target-dir` pointed at
the main checkout) and it silently mixed checkouts up. Cargo keys registry-dep
artifacts by package + features + profile + rustc, not by workspace path, so
those really are shareable — but local crates get a path-*independent*
`-C metadata` hash. Every checkout wrote the same
`deps/libodm_render-<hash>.rlib` and the same `.fingerprint/` entry, freshness
came down to source mtime, and a checkout whose sources predated a peer's last
build was judged fresh and linked the peer's code. Observed both as a build
failure (`cannot find function pick_wire in crate odm_render`, code that existed
locally) and, worse, as a binary quietly running another checkout's renderer.

Two lesser annoyances also go away with the split: cargo's exclusive lock on the
artifact dir no longer serializes builds across checkouts, and `cargo clean` no
longer wipes everyone's cache.

## Pruning stale artifacts

`scripts/sweep-target.py` (`--dry-run` to preview) sweeps the current checkout's
target dir. cargo never GCs one: every profile edit, dep bump, rustc upgrade, or
build-script rerun orphans the previous artifacts forever. Measured 2026-07-22:
15.7 GiB → 4.7 GiB, 4560 orphans, no live artifact lost.

Not `cargo-sweep`: it prunes by mtime, which can't distinguish live from orphaned
in a young target dir — `--time 1` here would have deleted ~everything, `--time
30` nothing. This instead asks cargo for the live set
(`cargo build --all-targets --message-format=json` → `compiler-artifact.filenames`
plus `build-script-executed.out_dir`, which yields the `-<hash>` of every live
unit) and deletes only unreferenced entries in `deps/`, `build/`, and
`.fingerprint/`. Over-deleting would only cost a rebuild, never break anything.
It enumerates dev *and* release (the sweep walks every profile dir; release
live-set comes from plain `--release`, no `--all-targets`, so it never builds
release test binaries just to enumerate them). `incremental/` can't be
liveness-matched (its dir suffix hash appears nowhere in fingerprints or
artifact names, and fresh builds don't touch live dirs — both verified), and
idle-time pruning is exactly wrong there: every stale universe's dir is
recent, so churn-heavy weeks grew incremental/ to 5.4 GiB while everything
sat under any sane age cutoff. Instead the sweep keeps the newest 2 dirs per
crate name (current lib + test units) and drops older siblings;
`--drop-incremental` drops it all. Over-deleting incremental only slows that
crate's next recompile; it's an accelerator, not a freshness input.

In a seeded checkout most of `deps/` is hardlinked, so sweeping there frees real
disk only for entries the seed source no longer holds.

Worst offender observed: 14 `build/v8-*` dirs and 4 × 196 MiB `libv8-*.rlib`.
Two of the variants trace to a `[profile.dev]` edit; the rest to the v8 build
script re-running and cascading a new `-C metadata` hash into the v8 rlib. Its
`rerun-if-env-changed` list includes `OUT_DIR`, `HOST`, `SCCACHE`, `CCACHE`,
`CLANG_BASE_PATH`, `V8_FROM_SOURCE`, so almost any env difference between
invocations orphans another ~200 MiB. Expect to re-run the sweep periodically.
(A seeded target dir does *not* trip this: cargo kept every build script fresh
across the copy.)

## Dynamic linking in dev builds (crates/odm-dylib)

Dev binaries link the workspace stack through one Rust dylib
(bevy_dylib pattern): `crates/odm-dylib` is `crate-type = ["dylib"]`,
re-exporting odm-cli/engine/export (+ build/js/kernel for feature parity).
The odm bin pulls it via the default `dynamic` feature; heavy integration-test
binaries and xtask via a plain dep + `use odm_dylib as _;`. Measured: `odm`
245 MB → 0.1 MB, test binaries 139 MB → 2–4 MB, one 284 MB .so relinked only
when workspace code changes; an odm-engine edit + `cargo build --workspace`
writes ~0.4 GiB (was ~2.5 GiB pre-dylib at line-tables, ~4 GiB before that).
Lib *unit*-test binaries stay static: they compile the crate itself under
cfg(test), a second instantiation that can't dedupe against the dylib's copy.

Hard-won constraints, in dependency order:

- **odm-dylib is workspace-EXCLUDED** (root Cargo.toml `exclude`; its manifest
  can't use `.workspace = true` inheritance because of that). Cargo only
  passes `-C prefer-dynamic` (= dynamic libstd) when a dylib is built as a
  *dependency*; built as a requested root (any `--workspace` build, were it a
  member), it gets static libstd and every consumer fails with "cannot
  satisfy dependencies so `std` only shows up once".
- **A dylib's output path is unhashed** (`deps/libodm_dylib.so`), so feature
  universes that were merely wasteful for rlibs are thrash (or, pre-exclusion,
  that same std error) here. odm-dylib therefore mirrors what workspace-wide
  resolution enables via dev-deps: `test-api-version` (odm-build's/odm-js's
  own tests) and kernel `wasm-uu` (leaked by odm-web). Symptom of a new
  mismatch: libodm_dylib.so relinks when alternating `-p` and `--workspace`
  builds — mirror the new feature into odm-dylib's deps.
- **Standalone runs need the rpath from crates/odm/build.rs** ($ORIGIN,
  $ORIGIN/deps, and the toolchain libdir for libstd.so — linking any Rust
  dylib forces libstd dynamic). `cargo run`/`cargo test` work regardless
  (LD_LIBRARY_PATH). The dev binary is only relocatable together with its .so.
- **Shipping stays static**: install.sh builds `-p odm --release
  --no-default-features` (verified: no NEEDED beyond system libs). That is a
  third feature universe; sweep-target.py enumerates it explicitly so a sweep
  doesn't GC the installed flavor's artifacts.

## mold / sccache: installed, deliberately unused

Both would force a full rebuild to adopt: `RUSTFLAGS` and `RUSTC_WRAPPER` are
cargo fingerprint inputs.

- **mold** buys little: rustc 1.93 already links with `rust-lld` by default on
  x86_64-unknown-linux-gnu (confirmed — `readelf -p .comment target/debug/odm`
  says `LLD`), and a touch-one-file rebuild+link of the `odm` binary is ~1s total.
  Link is not the bottleneck.
- **sccache** is redundant here: it accelerates *cold* compiles of identical
  inputs, and seeding already means deps cold-compile exactly once per machine,
  without a second copy in `~/.cache/sccache` (178 MiB of stale probe entries sit
  there now; sccache is off, so it's dead weight).

  It also can't cache much: a 2-crate probe executed only 3 of 12 rustc calls,
  9 non-cacheable ("missing input" ×6, "crate-type" ×2).

  Decisive, measured 2026-07-24 on a single-crate no-deps workspace: same path +
  wiped target dir → **hit**; byte-identical sources at a *different* path →
  **miss**; per-checkout `SCCACHE_BASEDIRS=<checkout root>` (the env var is
  plural; `SCCACHE_BASE_DIR` is silently ignored) → still **miss**. So sccache
  gives zero cross-checkout reuse for `crates/*` — exactly the units a fresh
  checkout has to build — while `RUSTC_WRAPPER` being a fingerprint input would
  cost one full rebuild of the 7.8 GiB target dir to adopt. Build scripts aren't
  rustc calls, so the v8 download and the Manifold/TBB cmake build wouldn't be
  cached either.

## wasm32 toolchain (web-export spikes)

`rustup target add wasm32-unknown-unknown` is done. The Manifold wasm lane
needs libc++ headers and wasm-ld; neither is installed system-wide (no
root), so both were extracted from Arch packages into
`~/.local/opt/wasm-cxx/` (`libcxx-headers/`, `wasm-ld`). `cargo xtask
build-web-template` finds that directory itself (see `shim_env`), so it and
`scripts/install.sh` need no wrapper env; building a `wasm-uu` crate any
other way still wants
`WASM_CXX_SHIM_LIBCXX_HEADERS=~/.local/opt/wasm-cxx/libcxx-headers` and
`WASM_CXX_SHIM_WASM_LD=~/.local/opt/wasm-cxx/wasm-ld` exported (or
`pacman -S libc++ lld` with root and drop both vars). Keep `MANIFOLD_CSG_NO_SCCACHE=1` (sccache stays
off on this machine). The sys build script clones manifold/Clipper2/
wasm-cxx-shim from GitHub on first build.
