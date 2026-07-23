//! Minimal column-major 4x4 f64 matrix helpers (element i = col*4 + row).

pub type Mat4 = [f64; 16];
pub type Vec3 = [f64; 3];

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[k * 4 + r] * b[c * 4 + k];
            }
            out[c * 4 + r] = sum;
        }
    }
    out
}

pub fn transform_point(m: &Mat4, p: Vec3) -> Vec3 {
    let w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
    let w = if w == 0.0 { 1.0 } else { w };
    [
        (m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]) / w,
        (m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]) / w,
        (m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]) / w,
    ]
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn length(v: Vec3) -> f64 {
    dot(v, v).sqrt()
}

pub fn normalize(v: Vec3) -> Vec3 {
    let l = length(v);
    if l == 0.0 { [0.0, 0.0, 1.0] } else { [v[0] / l, v[1] / l, v[2] / l] }
}

pub fn scale3(v: Vec3, s: f64) -> Vec3 {
    [v[0] * s, v[1] * s, v[2] * s]
}

/// Right-handed look-at view matrix.
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        s[0], u[0], -f[0], 0.0, //
        s[1], u[1], -f[1], 0.0, //
        s[2], u[2], -f[2], 0.0, //
        -dot(s, eye),
        -dot(u, eye),
        dot(f, eye),
        1.0,
    ]
}

/// Right-handed perspective, depth 0..1 (wgpu convention).
pub fn perspective(fov_y_rad: f64, aspect: f64, near: f64, far: f64) -> Mat4 {
    let sy = 1.0 / (fov_y_rad / 2.0).tan();
    let sx = sy / aspect;
    let mut m = [0.0; 16];
    m[0] = sx;
    m[5] = sy;
    m[10] = far / (near - far);
    m[11] = -1.0;
    m[14] = near * far / (near - far);
    m
}

/// Right-handed orthographic, depth 0..1.
pub fn orthographic(half_w: f64, half_h: f64, near: f64, far: f64) -> Mat4 {
    let mut m = IDENTITY;
    m[0] = 1.0 / half_w;
    m[5] = 1.0 / half_h;
    m[10] = 1.0 / (near - far);
    m[14] = near / (near - far);
    m
}

pub fn to_f32_cols(m: &Mat4) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            out[c][r] = m[c * 4 + r] as f32;
        }
    }
    out
}
