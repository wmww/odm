# Cannot screenshot the viewer for verification

Any change to `crates/odm-engine/src/viewer.rs` / `theme.rs` currently ships
unverified: there is no way for the agent to see the running UI.

- `grim` (1.5.0) fails with "compositor doesn't support the screen capture
  protocol". `wayland-info` connects fine and lists 40 globals, but neither
  `zwlr_screencopy_manager_v1` nor `ext_image_copy_capture_manager_v1` is
  advertised to this client — sway 1.12 runs as root, we connect as uid 1006
  via the `wayland-root` socket symlink.
- No Xwayland, so `import`/`xwd` are out.
- `odm render` exercises `odm-render` only; it never touches egui, so it
  can't catch UI regressions.

Only the 3D viewport has a headless path — the egui chrome has none.

Options:
- Headless egui frame dump: `egui-wgpu` can render into an offscreen target
  using the device `odm-render` already owns. A small `--ui-shot <png>` mode
  on the engine (or a dev-only test binary) would run one frame with a
  synthetic `Published` and write a PNG. Also usable as a UI snapshot test.
- Or `egui_kittest`, which does this upstream, at the cost of a dev-dependency.
