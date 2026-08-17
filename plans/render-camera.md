# Render camera: one parameter set (`look`, `focus`, `zoom`, echo)

## Why
Live test friction: orthographic check views mean retyping
`{"direction": [0,0,-1], "ortho": true}` all session. And the camera
surface is two disjoint modes — auto (`direction`, placement computed,
`fov` silently ignored) vs explicit (`eye`+`target`, nothing computed,
`ortho_height` falling back to a hardcoded 10.0) — the classic
simple-API + bolted-on-complex-API grabbag. Rethink so the simple case
is the complex case with defaults.

## One parameter set, no modes
A render camera is fully described by: **target** (look-at point),
**direction** (of gaze), **distance**, **up**, **projection**
(ortho/perspective), **fov** / **ortho_height**. Every parameter is
independently either given in the request or defaulted, and the
defaults are what "auto framing" used to be: computed by fitting the
focused bounds. Resolution is one overlay pipeline — compute defaults
from bounds, overlay the request's fields — replacing the
`Camera::Auto | Explicit` enum in `camera.rs` and `camera_from` in
`commands.rs`.

Combinations that are dead today become meaningful:

- `{"eye": [60,-80,40]}` alone — camera there, looking at the (focused)
  model center; target no longer required.
- `{"eye": …, "ortho": true}` — ortho height defaulted from the fit,
  not 10.0.
- `{"fov": 20}` alone — long-lens framed overview; fit distance adapts
  to the fov instead of assuming 45°.

Over-determined combos are errors, not precedence puzzles: `eye`+`look`,
`eye`+`zoom`.

## Request fields
- `"look": "top"|"bottom"|"front"|"back"|"left"|"right"` — the six
  axis-aligned **orthographic** drafting views (wanting a drafting view
  is why you type the word). Keywords set direction *and* default the
  projection to ortho.
- `"look": [x, y, z]` — gaze along that vector, perspective. Keyword vs
  vector unambiguous by JSON type. Replaces `direction` (deleted; the
  spec table's removed-field error redirects). Why names at all when the
  vector exists: a keyword typo fails loudly, a sign error in the vector
  renders the wrong side silently — and keywords match user chat ("the
  front looks wrong").
- No name for the default. The no-camera-fields default stays the framed
  perspective overview along the deliberately skewed `DEFAULT_DIR`
  (equal-angle `[-1,-1,-1]` is the degenerate case: coincident projected
  edges on axis-aligned models).
- `ortho` becomes tri-state (`Option<bool>`): explicit beats implied, so
  `{"look": "top", "ortho": false}` is a perspective top-down and
  vector-`look` + `ortho: true` an ortho view from an arbitrary angle.
- `"focus": "seat"` — frame that node's subtree bounds (addressed as
  `inspect` addresses nodes), rest of the scene still drawn. The middle
  ground between whole-scene and explicit placement; agents think in
  parts, not coordinates.
- `"zoom": k` — factor on the fitted distance/height (2 = twice as
  close).
- Kept, docs-only: `eye`, `target`, `up`, `fov`, `ortho_height` — now
  ordinary parameters in the one set, each usable alone.

## Echo the resolved camera
Every render response reports the camera it actually used — resolved
`eye`/`target`/`up` plus `fov` or `ortho_height`, the same spelling the
request accepts. "Slightly to the left" = nudge the echoed numbers and
paste back; one `look: "front"` render teaches the keyword→axis mapping
by example.

## No camera adoption from `view` (reversal)
Earlier revision adopted the viewer tab's camera when a render targets
`view`. Dropped on principle (see design-decisions.md "User state:
sent, not sampled"): the camera at tool-call time is noise — users
orbit fast. `view: true` keeps meaning path+inputs only. Instead, the
**poll snapshot gains the camera**, in the same explicit spelling the
render request accepts/echoes, so the agent replays exactly the view a
message was about by pasting numbers. While there: stamp the snapshot
at message-send time (today `cmd_poll` samples at collect time — same
race, and a poll can collect long after the send). The viewer camera is
orbit-form (`tabs.rs` `SavedCamera`: target/distance/yaw/pitch);
convert to eye/target at the boundary.

## Prompt/docs
Prompt teaches the `look` keywords only (per `notes/agent-surface.md`;
promotion justified: replaces the `direction`+`ortho` pair the prompt
teaches today). `focus`/`zoom`/explicit fields are docs-only. The field
list in `docs/cli.md` is generated from the spec table in
`requests.rs`; the manual prose (docs/cli.md render section,
`docs/prompts/cli.md` examples) updates by hand
(`{"direction": [0,0,-1], "ortho": true}` → `{"look": "top"}`).

## Notes
- `render-frames.md` builds on this (per-frame `look`; its shared union
  fit is "compute the default fit once from the union"). Land this
  first.
- Camera nodes (cameras defined in JS, surviving sessions) are shelved
  as `plans/camera-nodes.md`; the overlay pipeline is what makes them
  cheap later — a node is just another source of defaults.
- Project-level named views in `odm.toml`: standing no — a `frames`
  sheet of the keyword views is one request.
- Judgment call, revisit if it bites: `look` keywords flip projection by
  spelling (keyword=ortho, vector=perspective). Tri-state `ortho` is
  the escape hatch in both directions.
