# Default window is larger than small screens

`viewer/idle.rs` opens at a fixed 1280×840 inner size. On the Windows CI
runner's 1024×768 desktop (2026-09-23 screenshot) the right-hand panels and
the window's own controls land off-screen; a 1366×768 laptop would lose the
bottom. Clamp the first-run size to the monitor (e.g. ≤ 90% of it) — egui's
`ViewportBuilder` has no monitor query before the window exists, so this
likely means reading it in the first frame and resizing once.
