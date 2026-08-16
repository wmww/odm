# Render camera: `look` + adopting the viewer camera

## Why
Live test friction: orthographic check views meant retyping
`--direction 0,0,-1 --ortho` all session. And `render` with `view` adopts
the tab's path + inputs but renders from the default angle — not what the
user is actually seeing.

## `look` subsumes `direction` + `ortho`
One camera field in the agent-visible surface:

- `"look": "top"|"bottom"|"front"|"back"|"left"|"right"` — axis-aligned
  **and** orthographic; wanting a drafting view is why you type the word.
- `"look": [x, y, z]` — arbitrary direction, perspective (replaces
  `direction`). Keyword vs vector is unambiguous by JSON type.
- Default stays the framed iso perspective; `"iso"` accepted as its name.

`direction` is deleted; `ortho` survives docs-only as a modifier for the
vector form, alongside the docs-only explicit five
(`eye`/`target`/`up`/`fov`/`ortho_height`). Named `look` because `view` is
taken (tab adoption). Spell it as a request field from the start — land
on the landed JSON grammar so no flag form ever exists.

## Adopt the user's camera
The diet's "target what the user sees" should include the camera. Tabs
already persist one (`viewer/tabs.rs` `SavedCamera`); when a render adopts
`view`, build a `Camera::Explicit` from the tab's camera unless the request
carries `look`/camera fields. A slot with no saved camera (fresh headless)
falls back to auto framing as now.

## Prompt/docs
Prompt teaches the `look` keywords only (per `notes/agent-surface.md`,
promotion is justified: it replaces the `--direction`+`--ortho` pair the
prompt teaches today). Update `docs/prompts/cli.md` examples
(`--direction 0,0,-1 --ortho` → `"look": "top"`) and `docs/cli.md`.

## Notes
- Project-level named views in `odm.toml` (presets-for-cameras): still not
  needed — the keywords cover what the test session actually typed.
- Judgment call, revisit if it bites: keywords imply ortho with no
  perspective escape hatch in the prompt surface (vector form + docs-only
  `ortho` cover the rare cases).
