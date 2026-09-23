# Plan: macOS support, remaining step

Everything else in the port is done (2026-09-23, see notes/architecture.md
"Platforms ▸ macOS" and build-environment.md "macOS lane"): the `macos` lane
runs `cargo test --workspace` green, the viewer was checked on the runner.

Left, blocked on `plans/windows.md` merging (it owns the files):

- Rebase on the Windows merge, then on the macos lane verify:
  - the stdin-EOF contract test in `odm-agent/tests/client.rs` (a killed
    engine on macOS relies on the adapters exiting on EOF — no PDEATHSIG);
  - the exact-name import check (`windows.md` item 5) on case-insensitive
    APFS;
  - the `state.rs` watcher test under FSEvents (coalesced events; may need
    the `ODM_TEST_TIMEOUT_SCALE` treatment);
  - `viewer_smoke` on both the windows and macos lanes.
- Record the results in the macOS notes, then delete this plan.
