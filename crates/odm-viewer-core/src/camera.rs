//! The viewport's orbit camera.

use odm_render::math::{cross, normalize};
use odm_render::{Camera, FIT_MARGIN, fit_distance};

/// Vertical field of view every viewport camera uses — also what hosts quote
/// when they describe the camera (e.g. the chat view snapshot).
pub const FOV_Y_DEG: f64 = 45.0;

/// Orbit camera: spherical eye around a target, Z-up.
pub struct Orbit {
    pub target: [f64; 3],
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
}

impl Orbit {
    pub fn framed(bounds: Option<([f64; 3], [f64; 3])>) -> Orbit {
        let mut orbit = Orbit {
            target: [0.0; 3],
            distance: 1.0,
            yaw: 1.4f64.atan2(1.0),
            pitch: 0.9f64.atan2((1.0f64 + 1.4 * 1.4).sqrt()),
        };
        orbit.frame(bounds, 1.0);
        orbit
    }

    /// Center on some bounds and back off just far enough that every box
    /// corner fits the viewport (both fov axes, per-corner depth), keeping
    /// the view direction — reframing should not spin the model.
    pub fn frame(&mut self, bounds: Option<([f64; 3], [f64; 3])>, aspect: f64) {
        let fallback = FIT_MARGIN / (FOV_Y_DEG / 2.0).to_radians().sin();
        let Some((min, max)) = bounds else {
            self.target = [0.0; 3];
            self.distance = fallback;
            return;
        };
        self.target =
            [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, (min[2] + max[2]) / 2.0];
        let tan_y = (FOV_Y_DEG / 2.0).to_radians().tan();
        let dist =
            fit_distance((min, max), self.target, self.basis(), tan_y * aspect.max(1e-3), tan_y);
        // Degenerate bounds (a point) fall back like an empty scene.
        self.distance = if dist > 1e-9 { dist } else { fallback };
    }

    pub fn eye(&self) -> [f64; 3] {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        [
            self.target[0] + self.distance * cp * cy,
            self.target[1] + self.distance * cp * sy,
            self.target[2] + self.distance * sp,
        ]
    }

    pub fn camera(&self) -> Camera {
        Camera {
            eye: Some(self.eye()),
            target: Some(self.target),
            up: Some([0.0, 0.0, 1.0]),
            fov_y_deg: Some(FOV_Y_DEG),
            ..Camera::default()
        }
    }

    /// Camera basis (right, up, forward), for panning and picking.
    pub(crate) fn basis(&self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let eye = self.eye();
        let f = normalize([
            self.target[0] - eye[0],
            self.target[1] - eye[1],
            self.target[2] - eye[2],
        ]);
        let s = normalize(cross(f, [0.0, 0.0, 1.0]));
        let u = cross(s, f);
        (s, u, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odm_render::math::dot;

    /// A stick along x, viewed with the given angles at the given aspect.
    fn frame_stick(yaw: f64, pitch: f64, aspect: f64) -> Orbit {
        let mut orbit = Orbit { target: [0.0; 3], distance: 1.0, yaw, pitch };
        orbit.frame(Some(([-5.0, -0.1, -0.1], [5.0, 0.1, 0.1])), aspect);
        orbit
    }

    const TAN_Y: f64 = 0.41421356237309503; // tan(22.5°)

    #[test]
    fn side_on_stick_fills_a_wide_viewport() {
        // Eye on +y (yaw 90°): the stick spans the horizontal fov, so the
        // fit is against tan_x, plus the thickness's depth offset.
        let orbit = frame_stick(std::f64::consts::FRAC_PI_2, 0.0, 2.0);
        let want = 5.0 * 1.1 / (TAN_Y * 2.0) + 0.1;
        assert!((orbit.distance - want).abs() < 1e-9, "{} vs {want}", orbit.distance);
    }

    #[test]
    fn dead_on_stick_fits_its_thickness_not_its_length() {
        // Eye on +x (yaw 0): only the 0.1 cross-section faces the camera;
        // the length is depth (the near end pushes the eye back 5).
        let orbit = frame_stick(0.0, 0.0, 2.0);
        let want = 0.1 * 1.1 / TAN_Y + 5.0;
        assert!((orbit.distance - want).abs() < 1e-9, "{} vs {want}", orbit.distance);
    }

    #[test]
    fn framing_keeps_the_view_direction_and_centers() {
        let orbit = frame_stick(0.7, 0.4, 1.5);
        assert_eq!(orbit.target, [0.0, 0.0, 0.0]);
        assert_eq!((orbit.yaw, orbit.pitch), (0.7, 0.4));
        // Every corner projects inside both half-angles (with margin).
        let (s, u, f) = orbit.basis();
        for ix in [-5.0, 5.0] {
            for iy in [-0.1, 0.1] {
                for iz in [-0.1, 0.1] {
                    let v = [ix, iy, iz];
                    let depth = orbit.distance + dot(v, f);
                    assert!(dot(v, s).abs() <= TAN_Y * 1.5 * depth + 1e-9);
                    assert!(dot(v, u).abs() <= TAN_Y * depth + 1e-9);
                }
            }
        }
    }

    #[test]
    fn degenerate_bounds_fall_back() {
        let mut orbit = Orbit { target: [5.0; 3], distance: 9.0, yaw: 0.0, pitch: 0.0 };
        orbit.frame(Some(([1.0; 3], [1.0; 3])), 1.0);
        assert_eq!(orbit.target, [1.0; 3]);
        assert!((orbit.distance - 1.1 / (FOV_Y_DEG / 2.0).to_radians().sin()).abs() < 1e-12);
    }
}
