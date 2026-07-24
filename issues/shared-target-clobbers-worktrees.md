# Shared target dir: checkouts silently build each other's local crates

`scripts/shared-target.sh` points every worktree at the main checkout's
`target/`. Registry deps genuinely are path-independent and shared safely, but
**local crates are not**: all checkouts write the same artifact name and the
same `.fingerprint/` entry.

Measured 2026-07-24, three checkouts (main + two worktrees):

- 14:02 `cargo test` in `/home/ai/odm` wrote
  `target/debug/deps/libodm_render-4008ab007b89ff57.rlib`; its `.d` listed
  main's source set (no `wire.rs`).
- 14:04 `cargo build` in `.worktrees/wt_cVnolxypT0vr4Obc` wrote **the same
  filename**, `.d` now listing that worktree's source set (with `wire.rs`).

Dep-info paths are workspace-relative, so freshness comes down to source
mtimes. A checkout whose sources are older than another checkout's last build
is judged *fresh* and links the other checkout's rlib. Observed symptoms:

- `cargo build` fails with "cannot find function `pick_wire` in crate
  `odm_render`" — code that exists in this checkout, compiled against a peer's
  stale rlib.
- Worse when it doesn't fail: `cargo run` produces a binary running another
  checkout's renderer. Wireframe rendered the old shaded-plus-overlay style in
  a checkout that no longer contains that code.
- `touch`ing the sources fixes it until the next peer build. Two agents
  working concurrently re-break it for each other continuously.

Not a locking problem — cargo's exclusive lock serializes builds correctly.
The bug is that two different source trees share one artifact identity.

Options:

1. Per-worktree target dirs (drop the sharing). Correct, costs one cold build
   per worktree (~3.6 GiB, v8 download + Manifold cmake). What the sharing was
   introduced to avoid.
2. Per-worktree target dir seeded with hardlinks (`cp -al`) from main's, since
   dep artifacts *are* path-independent. Near-zero disk and no cold build, but
   any in-place rewrite (fingerprints, `.d` files) would corrupt the peer —
   needs verifying before trusting.
3. Keep sharing, single active checkout at a time. Free, but it is a
   convention nothing enforces, and violating it fails silently.

Until this is resolved: build in the checkout you are about to run, and treat
any cross-checkout build as a reason to `touch` sources and rebuild.
