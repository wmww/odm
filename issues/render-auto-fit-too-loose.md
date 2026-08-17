# Render auto-fit frames elongated objects too small

`Camera::resolve` (crates/odm-render/src/camera.rs) auto-fits the *bounding
sphere* of the framed bounds into the tighter fov half-angle. For elongated
or flat objects the sphere is much bigger than any actual screen extent, so
`odm render` auto-framing leaves them small — worst when looking down the
long axis.

The viewer's F-frame had the same problem and was fixed by fitting the 8
AABB corners against both fov axes with per-corner depth
(`Orbit::frame` in crates/odm-viewer-core/src/camera.rs). The same approach
would work in `resolve` (it knows bounds, direction, and aspect), but it
changes every auto-framed render's output, and the reported/fitted camera
spelling — do it deliberately, not as a drive-by.
