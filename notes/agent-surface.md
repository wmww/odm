# Agent surface policy (prompt vs docs)

Adopted 2026-08 (from plans/cli-diet.md, after the 2026-08 surface audit).
The scarce resource is the agent's context: prompt is ~3K tokens paid once
per session, but output *shape* dominates everything (the un-dieted `tree`
printed 191 KB). Priority order for cuts: output fields ≫ concepts ≫
command count.

## The policy

- **The prompt teaches only the core loop**: edit → `inspect` → `render` →
  `poll`/`say`, plus `--set`/`--preset` and "parts have names". After basic
  work the concept load should be: files are doohickeys; queries target a
  view; `--set` sets inputs; parts have names; poll/say to talk.
- **Everything else lives in `odm docs`**, paid for only when consulted.
  `docs/cli.md` (topic `cli`) is the full CLI reference: raycast, explicit
  camera placement (`--eye/--target/--up/--fov/--ortho-height`), viewer
  view slots, index-path addressing, `--project`. Cascade mechanics,
  memoization, generations, color spaces, camera math — optional depth.
- **New features default to docs-only.** Promotion into the prompt needs a
  reason on the scale of the core loop.
- Cuts are about defaults and prompt space — full capability stays
  reachable via CLI + docs, and `odm --help` lists everything.

## Standing cuts (don't reintroduce)

- No standalone `sync` command: every command syncs first; `status` is the
  answer if the rescan is all you want. The CLI redirects the name.
- One "target what the user sees" flag: `--view` bare = active viewer tab,
  `--view <slot>` = a named slot (wire: `view: true | "slot"`). There is no
  `--viewer-state`.
- `generation` appears only in `status` (internal consistency counter).
- Nothing agent-visible prints content hashes.
