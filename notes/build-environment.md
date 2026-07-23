# Build environment

## Worktrees share the main checkout's target dir

`scripts/shared-target.sh` writes `<worktree>/.cargo/config.toml` with
`build.target-dir = "<main checkout>/target"`. It derives the main checkout from
`git rev-parse --git-common-dir`, is idempotent, no-ops in the main checkout (its
default `target/` *is* the shared one) and outside a git repo, and refuses to
clobber a hand-written config. `.cargo/` is gitignored.

Auto-applied by two hooks in the tracked `.claude/settings.json`: `SessionStart`
(covers fresh clones and existing worktrees) and `WorktreeCreate` (covers
worktrees made mid-session, which never fire SessionStart). Human devs can just
run the script.

Why sharing works: cargo keys registry-dep artifacts by package + features +
profile + rustc, *not* by workspace path, so all ~950 deps are shared. Local
crates get a path-dependent `-C metadata` hash and coexist without clobbering.

Measured 2026-07-22: fresh worktree build **5s** and ~0 disk growth, vs a cold
build of ~3.6 GiB. Builds in main and in a worktree stay no-ops for each other —
no fingerprint thrash.

Expensive shared pieces this reuses:
- prebuilt `librusty_v8.a` (177 MiB download) → `target/debug/gn_out/obj/`
- `manifold-csg-sys` cmake build of Manifold + TBB
- wgpu/naga/deno_core/egui rlibs (196 MiB v8, 77 MiB deno_core, 73 MiB ash, …)

Caveats: cargo takes an exclusive lock on the artifact dir, so simultaneous
builds across worktrees serialize ("Blocking waiting for file lock"). And
`cargo clean` from any worktree wipes the shared dir.

## Pruning stale artifacts

`scripts/sweep-target.py` (`--dry-run` to preview). cargo never GCs a target
dir: every profile edit, dep bump, rustc upgrade, or build-script rerun orphans
the previous artifacts forever. Measured 2026-07-22: 15.7 GiB → 4.7 GiB, 4560
orphans, no live artifact lost (all three checkouts still built as no-ops after).

Not `cargo-sweep`: it prunes by mtime, which can't distinguish live from orphaned
in a young target dir — `--time 1` here would have deleted ~everything, `--time
30` nothing. This instead asks cargo for the live set
(`cargo build --all-targets --message-format=json` → `compiler-artifact.filenames`
plus `build-script-executed.out_dir`, which yields the `-<hash>` of every live
unit) and deletes only unreferenced entries in `deps/`, `build/`, and
`.fingerprint/`. Over-deleting would only cost a rebuild, never break anything.
`incremental/` is dropped wholesale (it's a recompile accelerator, not an input
to freshness — dropping it doesn't even make the next build non-fresh).

Worst offender observed: 14 `build/v8-*` dirs and 4 × 196 MiB `libv8-*.rlib`.
Two of the variants trace to a `[profile.dev]` edit; the rest to the v8 build
script re-running and cascading a new `-C metadata` hash into the v8 rlib. Its
`rerun-if-env-changed` list includes `OUT_DIR`, `HOST`, `SCCACHE`, `CCACHE`,
`CLANG_BASE_PATH`, `V8_FROM_SOURCE`, so almost any env difference between
invocations orphans another ~200 MiB. Expect to re-run the sweep periodically.

## mold / sccache: installed, deliberately unused

Both would force a full rebuild of the shared cache to adopt: `RUSTFLAGS` and
`RUSTC_WRAPPER` are cargo fingerprint inputs.

- **mold** buys little: rustc 1.93 already links with `rust-lld` by default on
  x86_64-unknown-linux-gnu (confirmed — `readelf -p .comment target/debug/odm-engine`
  says `LLD`), and a touch-one-file rebuild+link of `odm-engine` is 0.95s total.
  Link is not the bottleneck.
- **sccache** is redundant here: it accelerates *cold* compiles of identical
  inputs, and a shared target dir already makes deps cold-compile exactly once
  per machine, without a second copy in `~/.cache/sccache`. It also can't cache
  everything (a trivial 1-crate probe: 0 of 6 rustc calls cacheable — "missing
  input", "crate-type"), and absolute source paths in the rustc command line
  differ per worktree, so local crates would miss anyway.
