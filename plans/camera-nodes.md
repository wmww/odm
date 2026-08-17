# Camera nodes (shelved)

Cameras as scene nodes, built in JS like any geometry — named, with the
same properties as the render camera parameter set (target, direction or
eye, projection, fov, …). Shelved until a live session wants a camera
that outlives one request. The overlay pipeline it builds on landed
2026-08-17 (notes/agent-surface.md, render camera section).

## Why it might earn its place
- A saved way to look at the model survives between agent sessions as
  code, and the user can use it too (viewer snaps a tab to a named
  camera).
- A camera's `build()` can read `t`: turntables and walkthrough shots
  fall out of the existing animation model, and compose with `frames`
  (one camera path, several moments).

## Shape
- An IR node type carrying camera parameters; shows up in `inspect`.
- Render request: `"camera": "<node name>"` (same addressing as
  everything else).
- Not a mode: a camera node is another *source of defaults* in the
  overlay pipeline. Precedence: request field > camera node property >
  fitted default — so `{"camera": "foo", "ortho": true}` renders foo's
  view orthographic, no new machinery.
- Never the default; `root.js` without cameras behaves exactly as
  today.

## Cost (why shelved)
IR node type + framework API + inspect/viewer treatment — real scope,
and nothing in the current workflow blocks on it.
