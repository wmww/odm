use crate::math::{self, Mat4, Vec3};

pub enum Camera {
    /// Auto-frame the scene bounds looking along `direction` (need not be
    /// unit length). Perspective 45° or fitted orthographic.
    Auto { direction: [f64; 3], ortho: bool },
    Explicit { eye: [f64; 3], target: [f64; 3], up: [f64; 3], projection: Projection },
}

pub enum Projection {
    Perspective { fov_y_deg: f64 },
    /// World-space height of the view volume.
    Orthographic { height: f64 },
}

impl Camera {
    /// Iso-ish view: from +x/+y/above, looking down at the scene.
    pub const DEFAULT_DIR: [f64; 3] = [-1.0, -1.4, -0.9];
}

pub(crate) struct ResolvedCamera {
    pub view_proj: Mat4,
    pub eye: Vec3,
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
    pub(crate) fn resolve(
        &self,
        bounds: Option<([f64; 3], [f64; 3])>,
        aspect: f64,
    ) -> ResolvedCamera {
        let (center, radius) = scene_sphere(bounds);
        let r = radius * FIT_MARGIN;
        match self {
            Camera::Auto { direction, ortho } => {
                let dir = math::normalize(*direction);
                let up = up_for(dir);
                if *ortho {
                    let (half_w, half_h) =
                        if aspect >= 1.0 { (r * aspect, r) } else { (r, r / aspect) };
                    let dist = r * 3.0;
                    let eye = math::sub(center, math::scale3(dir, dist));
                    let view = math::look_at(eye, center, up);
                    let proj = math::orthographic(half_w, half_h, dist - 2.0 * r, dist + 2.0 * r);
                    ResolvedCamera { view_proj: math::mul(&proj, &view), eye }
                } else {
                    let fov_y = 45f64.to_radians();
                    let fov_x = 2.0 * ((fov_y / 2.0).tan() * aspect).atan();
                    let half = fov_y.min(fov_x) / 2.0;
                    let dist = r / half.sin();
                    let eye = math::sub(center, math::scale3(dir, dist));
                    let near = (dist - 2.0 * r).max(dist / 100.0);
                    let far = dist + 4.0 * r;
                    let view = math::look_at(eye, center, up);
                    let proj = math::perspective(fov_y, aspect, near, far);
                    ResolvedCamera { view_proj: math::mul(&proj, &view), eye }
                }
            }
            Camera::Explicit { eye, target, up, projection } => {
                let dist = math::length(math::sub(*target, *eye)).max(1e-6);
                let reach = dist + 4.0 * r;
                let near = (dist - 2.0 * r).max(reach / 10_000.0);
                let view = math::look_at(*eye, *target, *up);
                let proj = match projection {
                    Projection::Perspective { fov_y_deg } => {
                        math::perspective(fov_y_deg.to_radians(), aspect, near, reach)
                    }
                    Projection::Orthographic { height } => {
                        let half_h = height / 2.0;
                        math::orthographic(half_h * aspect, half_h, near, reach)
                    }
                };
                ResolvedCamera { view_proj: math::mul(&proj, &view), eye: *eye }
            }
        }
    }
}
