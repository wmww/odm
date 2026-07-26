# Build environment

## Every checkout gets its own target dir, seeded from the main one

`scripts/seed-target.sh` gives a linked checkout its own `target/`, copied from
the main checkout's the first time it runs. Auto-applied by the `SessionStart`
hook in the tracked `.claude/settings.json`; idempotent, and a no-op in the main
checkout, outside a git repo, or once `target/` exists. Run it by hand if a
checkout is set up without a new session.

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
  checkout builds its own in 6 s.

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
`incremental/` is dropped wholesale (it's a recompile accelerator, not an input
to freshness — dropping it doesn't even make the next build non-fresh).

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
