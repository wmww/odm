use crate::math::{self, Mat4, Vec3};

/// A render camera: one set of independent parameters, each either given or
/// defaulted by fitting the framed bounds. "Auto framing" is just the
/// all-defaults case; an explicit camera is the same pipeline with fields
/// overlaid. Contradictory combinations (e.g. `eye` + `zoom`) are the
/// caller's job to reject — resolution itself has one precedence: given
/// beats fitted, and `zoom` scales only fitted values.
#[derive(Clone, Debug, Default)]
pub struct Camera {
    /// Camera position. Given alone, the camera looks at the fit center.
    pub eye: Option<Vec3>,
    /// Look-at point; default the framed bounds' center.
    pub target: Option<Vec3>,
    /// Gaze direction (need not be unit length); the eye is placed by
    /// fitting. Unused when `eye` is given (the gaze is eye→target then).
    pub direction: Option<Vec3>,
    /// Default Z-up, or Y-up when looking straight up/down.
    pub up: Option<Vec3>,
    pub ortho: bool,
    /// Perspective field of view (default 45°; the fit adapts to it).
    pub fov_y_deg: Option<f64>,
    /// World-space height of the ortho view volume (default fitted).
    pub ortho_height: Option<f64>,
    /// Factor on the fitted distance/height: 2 = twice as close.
    pub zoom: Option<f64>,
    /// Bounds to frame instead of the scene's (`focus`), as (min, max).
    pub fit: Option<([f64; 3], [f64; 3])>,
}

/// Every parameter concrete: what the render actually used, reportable in
/// the same spelling the request takes.
pub struct ResolvedCamera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub projection: Projection,
    pub view_proj: Mat4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    Perspective { fov_y_deg: f64 },
    /// World-space height of the view volume.
    Orthographic { height: f64 },
}

impl Camera {
    /// Iso-ish view: from +x/+y/above, looking down at the scene.
    /// Deliberately skewed — equal-angle [-1,-1,-1] projects the edges of
    /// axis-aligned models on top of each other.
    pub const DEFAULT_DIR: [f64; 3] = [-1.0, -1.4, -0.9];

    pub const DEFAULT_FOV_Y_DEG: f64 = 45.0;
}

/// Margin factor every auto-fit leaves around the framed bounds.
pub const FIT_MARGIN: f64 = 1.1;

/// Eye distance from `target` back along `f` so every corner of the box
/// projects inside both fov half-angle tangents (with `FIT_MARGIN`),
/// accounting for per-corner depth. 0.0 for degenerate (point) bounds —
/// callers fall back. Shared by render auto-fit and the viewer's F-frame so
/// both produce the same framing.
pub fn fit_distance(
    (min, max): ([f64; 3], [f64; 3]),
    target: Vec3,
    (s, u, f): (Vec3, Vec3, Vec3),
    tan_x: f64,
    tan_y: f64,
) -> f64 {
    let mut dist: f64 = 0.0;
    for corner in corners(min, max) {
        let v = math::sub(corner, target);
        // Forward depth relative to the target plane: a corner nearer
        // the camera (w < 0) needs proportionally more distance.
        let w = math::dot(v, f);
        let x = math::dot(v, s).abs() * FIT_MARGIN;
        let y = math::dot(v, u).abs() * FIT_MARGIN;
        dist = dist.max(x / tan_x - w).max(y / tan_y - w);
    }
    dist
}

/// Half-extents of the box about `target` on the right/up axes, with
/// `FIT_MARGIN` applied (the ortho counterpart of `fit_distance`).
fn fit_extents((min, max): ([f64; 3], [f64; 3]), target: Vec3, s: Vec3, u: Vec3) -> (f64, f64) {
    let (mut hx, mut hy) = (0.0f64, 0.0f64);
    for corner in corners(min, max) {
        let v = math::sub(corner, target);
        hx = hx.max(math::dot(v, s).abs());
        hy = hy.max(math::dot(v, u).abs());
    }
    (hx * FIT_MARGIN, hy * FIT_MARGIN)
}

fn corners(min: [f64; 3], max: [f64; 3]) -> impl Iterator<Item = Vec3> {
    (0..8).map(move |i| {
        [
            if i & 1 == 0 { min[0] } else { max[0] },
            if i & 2 == 0 { min[1] } else { max[1] },
            if i & 4 == 0 { min[2] } else { max[2] },
        ]
    })
}

/// Give a set of auto-fitted cameras rendering at one aspect (contact-sheet
/// tiles) equal apparent scale: resolve each, take the largest fitted world
/// half-height at the target plane, and set each camera's `zoom` to match
/// it. The corner fit is direction-dependent, so mixed `look`s would
/// otherwise frame each tile to its own tightest distance. Callers pass
/// only cameras whose framing is fitted (no `eye`/`zoom`, no `ortho_height`
/// when ortho) with a shared `fit`.
pub fn share_fitted_scale(cameras: Vec<&mut Camera>, aspect: f64) {
    let half_h: Vec<f64> = cameras
        .iter()
        .map(|c| {
            let r = c.resolve(None, aspect);
            match r.projection {
                Projection::Perspective { fov_y_deg } => {
                    math::length(math::sub(r.target, r.eye))
                        * (fov_y_deg.to_radians() / 2.0).tan()
                }
                Projection::Orthographic { height } => height / 2.0,
            }
        })
        .collect();
    let max = half_h.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 {
        return;
    }
    for (c, h) in cameras.into_iter().zip(half_h) {
        c.zoom = Some(h / max);
    }
}

/// Right/up axes for a view along `f` (same orthogonalization as `look_at`).
fn frame_axes(f: Vec3, up: Vec3) -> (Vec3, Vec3) {
    let s = math::normalize(math::cross(f, up));
    (s, math::cross(s, f))
}

fn scene_sphere(bounds: Option<([f64; 3], [f64; 3])>) -> (Vec3, f64) {
    match bounds {
        Some((min, max)) => {
            let center =
                [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, (min[2] + max[2]) / 2.0];
            let radius = math::length(math::sub(max, min)) / 2.0;
            (center, if radius > 1e-9 { radius } else { 1.0 })
        }
        None => ([0.0; 3], 1.0),
    }
}

fn up_for(dir: Vec3) -> Vec3 {
    // Z-up world; fall back when looking straight up/down.
    if math::dot(dir, [0.0, 0.0, 1.0]).abs() > 0.999 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, 1.0] }
}

impl Camera {
    /// Fill every ungiven parameter from a fit of the framed bounds
    /// (`fit`, else `bounds`), then build the matrices.
    pub fn resolve(&self, bounds: Option<([f64; 3], [f64; 3])>, aspect: f64) -> ResolvedCamera {
        let framed = self.fit.or(bounds);
        let (center, radius) = scene_sphere(framed);
        let r = radius * FIT_MARGIN;
        let zoom = self.zoom.unwrap_or(1.0);
        let fov_y_deg = self.fov_y_deg.unwrap_or(Camera::DEFAULT_FOV_Y_DEG);

        let target = self.target.unwrap_or(center);
        let eye = match self.eye {
            Some(eye) => eye,
            None => {
                let f = math::normalize(self.direction.unwrap_or(Camera::DEFAULT_DIR));
                let dist = if self.ortho {
                    // Placement only sets the clip range; the height frames.
                    r * 3.0
                } else {
                    let tan_y = (fov_y_deg.to_radians() / 2.0).tan();
                    let (s, u) = frame_axes(f, self.up.unwrap_or_else(|| up_for(f)));
                    let corner_fit = framed
                        .map(|b| fit_distance(b, target, (s, u, f), tan_y * aspect, tan_y))
                        .filter(|d| *d > 1e-9);
                    // Empty/degenerate bounds: fall back to fitting the unit
                    // sphere in the tighter half-angle.
                    corner_fit.unwrap_or_else(|| {
                        let fov_y = fov_y_deg.to_radians();
                        let fov_x = 2.0 * ((fov_y / 2.0).tan() * aspect).atan();
                        r / (fov_y.min(fov_x) / 2.0).sin()
                    }) / zoom
                };
                math::sub(target, math::scale3(f, dist))
            }
        };
        let up = self.up.unwrap_or_else(|| up_for(math::normalize(math::sub(target, eye))));

        let dist = math::length(math::sub(target, eye)).max(1e-6);
        let reach = dist + 4.0 * r;
        let near = (dist - 2.0 * r).max(reach / 10_000.0);
        let view = math::look_at(eye, target, up);
        let (projection, proj) = if self.ortho {
            let height = self.ortho_height.unwrap_or_else(|| {
                // Fitted: the box's projected extents fit both dimensions.
                let f = math::normalize(math::sub(target, eye));
                let (s, u) = frame_axes(f, up);
                let corner_fit = framed
                    .map(|b| {
                        let (hx, hy) = fit_extents(b, target, s, u);
                        2.0 * hy.max(hx / aspect)
                    })
                    .filter(|h| *h > 1e-9);
                corner_fit.unwrap_or(2.0 * if aspect >= 1.0 { r } else { r / aspect }) / zoom
            });
            let half_h = height / 2.0;
            (
                Projection::Orthographic { height },
                math::orthographic(half_h * aspect, half_h, near, reach),
            )
        } else {
            (
                Projection::Perspective { fov_y_deg },
                math::perspective(fov_y_deg.to_radians(), aspect, near, reach),
            )
        };
        ResolvedCamera { eye, target, up, projection, view_proj: math::mul(&proj, &view) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Option<([f64; 3], [f64; 3])> = Some(([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]));

    #[test]
    fn defaults_frame_the_bounds() {
        let cam = Camera::default().resolve(BOUNDS, 4.0 / 3.0);
        // Looking at the center from the default direction, perspective 45°.
        assert_eq!(cam.target, [0.0, 0.0, 0.0]);
        assert!(matches!(cam.projection, Projection::Perspective { fov_y_deg } if fov_y_deg == 45.0));
        let dir = math::normalize(math::sub(cam.target, cam.eye));
        let want = math::normalize(Camera::DEFAULT_DIR);
        for k in 0..3 {
            assert!((dir[k] - want[k]).abs() < 1e-12, "{dir:?} vs {want:?}");
        }
    }

    #[test]
    fn eye_alone_looks_at_the_center() {
        let mut spec = Camera::default();
        spec.eye = Some([10.0, 0.0, 0.0]);
        let cam = spec.resolve(Some(([0.0, 0.0, 0.0], [2.0, 2.0, 2.0])), 1.0);
        assert_eq!(cam.eye, [10.0, 0.0, 0.0]);
        assert_eq!(cam.target, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn zoom_scales_the_fitted_distance_only() {
        let far = Camera::default().resolve(BOUNDS, 1.0);
        let mut spec = Camera::default();
        spec.zoom = Some(2.0);
        let near = spec.resolve(BOUNDS, 1.0);
        let d = |c: &ResolvedCamera| math::length(math::sub(c.target, c.eye));
        assert!((d(&far) / d(&near) - 2.0).abs() < 1e-12);

        // Explicit eye is never scaled.
        spec.zoom = None;
        spec.eye = Some([5.0, 5.0, 5.0]);
        assert_eq!(spec.resolve(BOUNDS, 1.0).eye, [5.0, 5.0, 5.0]);
    }

    #[test]
    fn ortho_height_fits_or_obeys() {
        let mut spec = Camera::default();
        spec.ortho = true;
        spec.direction = Some([0.0, 0.0, -1.0]);
        let ortho_height = |cam: &ResolvedCamera| match cam.projection {
            Projection::Orthographic { height } => height,
            _ => panic!("not ortho"),
        };
        // Top-down over the unit cube: projected half-extents 1.1 each.
        // Wide frame: the height is the constraint.
        let cam = spec.resolve(BOUNDS, 2.0);
        assert!((ortho_height(&cam) - 2.2).abs() < 1e-12);
        // Tall frame: the width is the constraint, so height grows.
        let cam = spec.resolve(BOUNDS, 0.5);
        assert!((ortho_height(&cam) - 4.4).abs() < 1e-12);
        // Explicit height wins, zoom scales only the fitted one.
        spec.ortho_height = Some(7.0);
        spec.zoom = Some(2.0);
        let cam = spec.resolve(BOUNDS, 2.0);
        assert!(ortho_height(&cam) == 7.0);
        spec.ortho_height = None;
        let cam = spec.resolve(BOUNDS, 2.0);
        assert!((ortho_height(&cam) - 1.1).abs() < 1e-12);
        // The fit is the box's projection, not its bounding sphere: an
        // elongated slab framed down its long axis stays tight.
        spec.zoom = None;
        // Width 11 must fit in aspect 2 -> height 5.5, not sphere-sized.
        let cam = spec.resolve(Some(([-5.0, -1.0, -1.0], [5.0, 1.0, 1.0])), 2.0);
        assert!((ortho_height(&cam) - 5.5).abs() < 1e-12);
        spec.direction = Some([1.0, 0.0, 0.0]);
        let cam = spec.resolve(Some(([-5.0, -1.0, -1.0], [5.0, 1.0, 1.0])), 2.0);
        assert!((ortho_height(&cam) - 2.2).abs() < 1e-12);
    }

    #[test]
    fn perspective_fits_box_corners_not_the_sphere() {
        // A stick along x, viewed top-down at aspect 2 (the same case the
        // viewer's F-frame tests pin): the length spans the horizontal fov,
        // the thickness's near face adds its depth.
        let mut spec = Camera::default();
        spec.direction = Some([0.0, 0.0, -1.0]);
        let cam = spec.resolve(Some(([-5.0, -0.1, -0.1], [5.0, 0.1, 0.1])), 2.0);
        let tan_y = (45.0f64 / 2.0).to_radians().tan();
        let want = 5.0 * 1.1 / (tan_y * 2.0) + 0.1;
        let dist = math::length(math::sub(cam.target, cam.eye));
        assert!((dist - want).abs() < 1e-9, "{dist} vs {want}");
        // Viewed down the long axis only the cross-section faces the camera;
        // the near end pushes the eye back by the half-length.
        spec.direction = Some([-1.0, 0.0, 0.0]);
        let cam = spec.resolve(Some(([-5.0, -0.1, -0.1], [5.0, 0.1, 0.1])), 2.0);
        let want = 0.1 * 1.1 / tan_y + 5.0;
        let dist = math::length(math::sub(cam.target, cam.eye));
        assert!((dist - want).abs() < 1e-9, "{dist} vs {want}");
    }

    #[test]
    fn shared_scale_equalizes_mixed_looks() {
        // A stick framed side-on vs dead-on: very different tight fits.
        // After sharing, both show the same world half-height (the looser
        // side-on one), so a contact sheet's tiles read at one scale.
        let stick = Some(([-5.0, -0.1, -0.1], [5.0, 0.1, 0.1]));
        let mut side = Camera { direction: Some([0.0, 0.0, -1.0]), fit: stick, ..Camera::default() };
        let mut dead_on =
            Camera { direction: Some([-1.0, 0.0, 0.0]), fit: stick, ..Camera::default() };
        let half_h = |c: &Camera| {
            let r = c.resolve(None, 2.0);
            math::length(math::sub(r.target, r.eye)) * (22.5f64).to_radians().tan()
        };
        let loose = half_h(&side);
        assert!(half_h(&dead_on) < loose * 0.9, "fits should differ before sharing");
        share_fitted_scale(vec![&mut side, &mut dead_on], 2.0);
        assert!((half_h(&side) - loose).abs() < 1e-9);
        assert!((half_h(&dead_on) - loose).abs() < 1e-9);

        // Ortho tiles share through their height on the same scale.
        let mut ortho = Camera {
            direction: Some([-1.0, 0.0, 0.0]),
            ortho: true,
            fit: stick,
            ..Camera::default()
        };
        share_fitted_scale(vec![&mut side, &mut ortho], 2.0);
        match ortho.resolve(None, 2.0).projection {
            Projection::Orthographic { height } => assert!((height / 2.0 - loose).abs() < 1e-9),
            _ => panic!("not ortho"),
        }
    }

    #[test]
    fn fit_bounds_override_the_scenes() {
        let mut spec = Camera::default();
        spec.fit = Some(([4.0, 4.0, 4.0], [6.0, 6.0, 6.0]));
        let cam = spec.resolve(BOUNDS, 1.0);
        assert_eq!(cam.target, [5.0, 5.0, 5.0]);
    }

    #[test]
    fn up_defaults_by_gaze() {
        let mut spec = Camera::default();
        spec.direction = Some([0.0, 0.0, -1.0]);
        assert_eq!(spec.resolve(BOUNDS, 1.0).up, [0.0, 1.0, 0.0]);
        spec.direction = Some([1.0, 0.0, 0.0]);
        assert_eq!(spec.resolve(BOUNDS, 1.0).up, [0.0, 0.0, 1.0]);
        spec.up = Some([0.0, 1.0, 0.0]);
        assert_eq!(spec.resolve(BOUNDS, 1.0).up, [0.0, 1.0, 0.0]);
    }
}
