# Render workflow: contact sheets + camera shorthands

## Why
Live test friction: checking the swing animation meant three separate renders
at `t=0.5/1/1.5`, opened one at a time; and orthographic check views meant
retyping `--direction 0,0,-1 --ortho` etc. all session.

## Contact sheet / multi-frame
One render invocation producing one tiled PNG, frames labeled with their input
values. Motion reads better side by side, and it's one artifact to open.
Two input forms (pick one or both):

- explicit values: `--set t=0,0.5,1,1.5` (multi-value set for one name), or
- `--frames N`: sample a ranged cascade input across its declared min/max
  (`t` is the canonical case; range comes from the winning declaration).

Layout: auto grid, value captioned under each tile. Non-animation use falls
out for free: sweeping any input (`--set radius=8,10,12`) for comparison.

## Camera shorthands
`--look top|front|side|iso` for the axis-aligned ortho + default-iso cases
(sugar over `Camera::Auto{direction, ortho}` in `commands.rs`). Note
`--view` is taken (viewer-tab adoption; since the cli-diet merge it is the
one such flag, bare or with a slot), hence `--look`.

Optionally later: project-level named views in `odm.toml` (the `meta.presets`
idea applied to cameras). Not needed for v1 — the four built-ins cover what
the test session actually typed.

## Prompt/docs
Prompt teaches `--look` + `--direction` + contact sheets; the explicit camera
five-some (`--eye/--target/--up/--fov/--ortho-height`) is already docs-only
(`docs/cli.md`, per `notes/agent-surface.md`).
