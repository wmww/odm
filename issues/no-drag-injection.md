# UI testing can't inject drags, so orbit and pan go untested

`scripts/ui-shot.sh -a` drives the viewer with `wdotool` (wlr-protocols
backend). Clicks, scrolls and keys work; drags do not, so the viewport's orbit
(primary drag) and pan (shift/middle drag) paths in
`crates/odm-engine/src/viewer.rs` `viewport_ui` can only be checked by eye.

Cause: each `wdotool` invocation creates and destroys its own virtual pointer,
and the compositor releases the button when the device goes. So
`mousedown 1` / `mousemove` / `mouseup 1` as three calls arrive at the app as a
single click at the press point. `wdotool replay` looked like a way out — one
process, one device, a whole trace — but its `RecEvent::Click` is
`{t_ms, button}` with no press/release flag, i.e. press+release are atomic.
(Its `move_abs` also appears not to reach the app at all, untested further.)

Modifier-clicks are out for the same reason: a `keydown Shift_L` is released
when its call exits, so the following click arrives unmodified (2026-07-26).

Options, roughly in order of appeal:

- An in-process egui frame dump (`egui_kittest`, or an engine `--ui-shot` mode)
  that feeds synthetic events straight into egui. That is the answer for
  deterministic UI snapshot *tests* anyway, and it sidesteps the compositor.
  `viewer::tests` already does the non-snapshot half of this by hand — it runs
  the scene tree in a bare `egui::Context` and clicks real row rects, which is
  how shift-select is tested.
- A tiny wlr virtual-pointer helper of our own that holds one device open and
  reads ops from stdin — perhaps 100 lines of `wayland-client`.
- Upstream: a `--hold`/`press`/`release` pair in wdotool that keeps the device
  alive, or a press-state flag on `RecEvent::Click` so `replay` can express a
  drag.

Background and the other wdotool quirks are in `notes/architecture.md`
("Seeing the viewer").
