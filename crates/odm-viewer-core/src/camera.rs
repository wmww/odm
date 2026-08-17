//! The viewport's orbit camera.

use odm_render::Camera;
use odm_render::math::{cross, normalize};

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
        orbit.frame(bounds);
        orbit
    }

    /// Center on some bounds and back off far enough to fit them, keeping the
    /// view direction — reframing should not spin the model.
    pub fn frame(&mut self, bounds: Option<([f64; 3], [f64; 3])>) {
        let (center, radius) = match bounds {
            Some((min, max)) => {
                let c = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, (min[2] + max[2]) / 2.0];
                let d = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
                let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 2.0;
                (c, if r > 1e-9 { r } else { 1.0 })
            }
            None => ([0.0; 3], 1.0),
        };
        self.target = center;
        self.distance = radius * 1.1 / (FOV_Y_DEG / 2.0).to_radians().sin();
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
