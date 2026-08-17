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

const FIT_MARGIN: f64 = 1.1;

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
        let (center, radius) = scene_sphere(self.fit.or(bounds));
        let r = radius * FIT_MARGIN;
        let zoom = self.zoom.unwrap_or(1.0);
        let fov_y_deg = self.fov_y_deg.unwrap_or(Camera::DEFAULT_FOV_Y_DEG);

        let target = self.target.unwrap_or(center);
        let eye = match self.eye {
            Some(eye) => eye,
            None => {
                let dir = math::normalize(self.direction.unwrap_or(Camera::DEFAULT_DIR));
                let dist = if self.ortho {
                    // Placement only sets the clip range; the height frames.
                    r * 3.0
                } else {
                    // Fit the bounding sphere in the tighter half-angle.
                    let fov_y = fov_y_deg.to_radians();
                    let fov_x = 2.0 * ((fov_y / 2.0).tan() * aspect).atan();
                    let half = fov_y.min(fov_x) / 2.0;
                    r / half.sin() / zoom
                };
                math::sub(target, math::scale3(dir, dist))
            }
        };
        let up = self.up.unwrap_or_else(|| up_for(math::normalize(math::sub(target, eye))));

        let dist = math::length(math::sub(target, eye)).max(1e-6);
        let reach = dist + 4.0 * r;
        let near = (dist - 2.0 * r).max(reach / 10_000.0);
        let view = math::look_at(eye, target, up);
        let (projection, proj) = if self.ortho {
            let height = self.ortho_height.unwrap_or_else(|| {
                // Fitted: the sphere fits both dimensions.
                2.0 * (if aspect >= 1.0 { r } else { r / aspect }) / zoom
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
        // r = sqrt(3) * 1.1; wide frame: height = 2r.
        let cam = spec.resolve(BOUNDS, 2.0);
        let r = 3f64.sqrt() * 1.1;
        match cam.projection {
            Projection::Orthographic { height } => assert!((height - 2.0 * r).abs() < 1e-12),
            _ => panic!("not ortho"),
        }
        // Tall frame: the width is the constraint, so height grows.
        let cam = spec.resolve(BOUNDS, 0.5);
        match cam.projection {
            Projection::Orthographic { height } => assert!((height - 4.0 * r).abs() < 1e-12),
            _ => panic!("not ortho"),
        }
        // Explicit height wins, zoom scales only the fitted one.
        spec.ortho_height = Some(7.0);
        spec.zoom = Some(2.0);
        let cam = spec.resolve(BOUNDS, 2.0);
        assert!(matches!(cam.projection, Projection::Orthographic { height } if height == 7.0));
        spec.ortho_height = None;
        let cam = spec.resolve(BOUNDS, 2.0);
        match cam.projection {
            Projection::Orthographic { height } => assert!((height - r).abs() < 1e-12),
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
