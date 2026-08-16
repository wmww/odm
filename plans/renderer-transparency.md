# Renderer restructure: transparency without special cases

## Why
The renderer has no transparency: `parseColor` rejects alpha ≠ 1, pipelines
have `blend: None`, `fs_mesh` hard-codes alpha 1.0. Agents want translucent
objects ("outer box at 20%") and whole-render x-ray ("everything at 30% so I
can see inside"). Beyond that, the current single-pass design encodes
compositing policy as per-pipeline depth-flag hacks (grid: test-but-don't-
write; wires: write-only-in-wireframe-mode) that interfere pairwise with
every new feature. This plan adds transparency by restructuring so
transparency, wireframe, grid, and future overlays (selection outlines that
shine through) all compose without hacks.

## Decided with the user (2026-08-15)
- **No sorting for correctness.** Blending order must not depend on draw
  order heuristics. Translucency via **depth peeling**: exact,
  order-independent, deterministic — fits the byte-equivalence invariant.
- **Drop MSAA entirely; no AA by default.** Verified empirically (AA/no-AA
  side-by-side renders): surface silhouettes are fine aliased and suit the
  retro look; the artifacts that actually hurt (grid moiré, dense-wireframe
  speckle) are *not* fixed by 4× MSAA and need shader-level treatment.
  Door left open via an optional integer `supersample` render option
  (default 1) — render k× larger, box-downsample at the end. Orthogonal to
  everything else. Validate k×width/height (the internal target, not the
  requested size) against the device max texture dimension.
- **The grid stops being special** — same kind of thing as a wireframe.
  Grid and wires render through one line-quad path as ordinary geometry.
  Grid lines get analytic AA (coverage alpha) + per-fragment density fade
  for minor lines, which makes them translucent — so they composite through
  the same peeling path as any translucent surface (option B: we're building
  exact transparency anyway, use it). "Wire behind grid" then resolves
  correctly with zero special flags.
- **Two categories only**: *opaque* (writes depth, any order) and
  *translucent* (exactly composited by the peeler). A later *overlay* pass
  (selection outlines, gizmos) composites on top reading opaque depth.
  Analytic AA is just one way a surface ends up translucent, not a third
  category. No geometry gets bespoke depth/blend state ever again.

## Design

### Pass list
1. **Opaque**: opaque instances → color + depth. Single-sample everything.
2. **Translucent peel**: the translucent set = translucent surfaces + all
   line geometry (grid, wires). N front-to-back peel layers (start N=4),
   each layer being *two* passes:
   - Geometry pass into a scratch layer texture (cleared transparent,
     blend **replace**): discard fragments at-or-nearer-than peel depth
     i−1 (sampled from a texture) or at-or-behind the opaque depth; depth
     test picks the nearest remaining fragment. Replace + depth write is
     what isolates the single nearest fragment — blending *during* this
     pass would be wrong (every fragment that passes the depth test blends,
     not just the final winner, double-compositing within one layer).
   - Fullscreen pass compositing the scratch layer *under* the accumulated
     premultiplied color.
   Two peel-depth textures ping-pong (can't sample the depth being
   written). Tail: one final unsorted geometry pass for layers past N,
   blending under the accumulation directly (order = draw order), so worst
   cases degrade gracefully instead of dropping geometry.
3. **Overlays** (future, not this plan): outline/gizmo composites sampling
   opaque depth for visible-vs-hidden styling.
4. **Downsample** when `supersample > 1`.

Deterministic throughout: fixed draw order, peeling exact per pixel;
equal-depth ties resolve by draw order, which is stable.

**Depth invariance is load-bearing**: peeling only works if the same
triangle produces bit-identical depths in every pass — a ±1ulp difference
and the already-peeled surface re-wins each layer (duplicated/darkened
layers). WebGPU guarantees position invariance only within one pipeline, so:
one peel geometry pipeline reused for all N layers (only bind groups
ping-pong), and `@invariant` on the position builtin so the tail pipeline
(different blend state) agrees too.

**Coplanar ties collapse to one layer, by design**: a fragment exactly at
the peel depth is discarded in every later layer *and* the tail, so
coincident translucent surfaces render once, not darker. This is a feature
— grid line crossings don't double-darken, z-fighting translucent faces
don't stipple — but note it: stacked identical translucent geometry does
not accumulate opacity.

### Lines
One line-quad path (today's `vs_wire` screen-space quads) serves wires and
grid; the 1px `Lines` line-primitive pipeline is deleted. `build_grid`'s
line lists survive, rendered as quad instances. Fragment shader computes
coverage alpha across the quad width (analytic AA) and, for grid minor
lines, fades alpha by projected line spacing so dense regions melt away
instead of moiré-ing. Wires become world geometry, correctly occluded and
composited; wireframe mode is still "skip the fill passes". A dimmed
hidden-lines wireframe style is a future overlay, not this plan.

### Alpha semantics (confirm at implementation)
- Node color stays RGBA with replace-wins inheritance; alpha now accepted
  (`[r,g,b,a]`, `#rrggbbaa`) — delete the parseColor guard.
- New optional node `opacity`, **multiplicative down the tree**: effective
  alpha = color.a × product of ancestor opacities. This is what "make this
  subassembly translucent" means; color replace-wins alone can't express
  it. Note this is *not* SVG group opacity (which isolates the group via
  offscreen compositing, hiding siblings from each other): here a 50%
  subassembly shows its internals through each other — the x-ray behavior
  we actually want, and far simpler. JS: `.opacity(x)` on scene values.
  IR: optional f32 on Node, hashes like color.
- X-ray: `RenderOptions.opacity` multiplier over all instances; an
  `opacity` field in the render request (JSON grammar); viewer menu
  toggle next to wireframe.
- Instance partition by effective alpha: 1 → opaque pass, <1 → translucent.
  All line geometry is translucent by construction (AA edges).

### PNG / background alpha
Peeling accumulates premultiplied color; composite over the background with
a proper "over" for the alpha channel, un-premultiply at readback. Makes
transparent-background PNGs correct (today dst alpha is accidental).

### Interface change
Renderer owns all intermediate targets (color, opaque depth, peel depths,
accumulation, supersampled variants); callers hand one final single-sample
texture view. Viewer (`viewer/mod.rs:640-720`) stops making its msaa/resolve
pair — keeps only its egui-registered texture. `render_png` unchanged
externally. Touches `plans/agent-activity-view.md`'s "normal UI render"
assumption only through this simpler signature.

## Steps (each leaves the tree green)
1. **Interface + no-MSAA**: renderer owns targets, single-sample color and
   depth, viewer/`render_png` on the new signature. Re-baseline goldens once
   (the no-AA look).
2. **Unify lines**: grid through the line-quad path, delete the `Lines`
   pipeline. Interim: hard-edged opaque grid that writes depth (already
   more correct than today).
3. **Peeling + transparency**: peel infra, translucent partition, alpha
   unlock in parseColor, node `opacity` cascade, render-request `opacity`,
   viewer toggle, PNG alpha correctness.
4. **Analytic AA for lines**: coverage alpha + grid density fade;
   reclassify all line geometry as translucent.
5. **Supersample option** (small; can trail).

## Testing
- Golden renders: translucent-over-opaque pixel assertions (blend math),
  x-ray mode, grid density fade vs the old moiré, wire-behind-grid,
  transparent-background PNG alpha values.
- Determinism: repeat-render byte equality with interpenetrating translucent
  objects (the case sorting gets wrong and peeling must not).
- Invariance canary: a single translucent surface must render identically at
  N=1 and N=4 (layers 2..N empty). Catches the re-peel failure mode — if
  depths aren't bit-identical across passes, the same surface composites
  once per layer and the image darkens with N.
- Perf sanity at CAD scale: translucent set drawn N+1 times is fine for
  target scenes; note measured numbers in spike-findings when done.

## Risks / notes
- wgpu depth-texture sampling (Depth32Float, non-comparison) is supported on
  all target backends; peel ping-pong avoids sample-while-write.
- Peel count N=4 + tail is a guess; validate on an "everything at 20%"
  x-ray of a dense assembly.
- Grid quad count (~5k instances worst case) is trivial for the instanced
  quad path.
