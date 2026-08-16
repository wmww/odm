//! Ground-plane grid at z=0 (Z-up), auto-sized from scene bounds.

pub struct GridLines {
    /// Interleaved xyz line-list vertices.
    pub minor: Vec<f32>,
    pub major: Vec<f32>,
    /// Spacing between minor lines, for the shader's density fade.
    pub minor_step: f64,
}

pub fn build_grid(bounds: Option<([f64; 3], [f64; 3])>) -> GridLines {
    // Extent in x/y that must be covered, centered on the origin.
    let reach = match bounds {
        Some((min, max)) => min[0]
            .abs()
            .max(max[0].abs())
            .max(min[1].abs())
            .max(max[1].abs())
            .max(1e-3),
        None => 5.0,
    } * 1.2;

    // Major spacing: power of ten giving roughly 10 major cells across.
    let major_step = 10f64.powf((reach * 2.0 / 10.0).log10().ceil());
    let minor_step = major_step / 10.0;
    let half = (reach / major_step).ceil() * major_step;

    let mut minor = Vec::new();
    let mut major = Vec::new();
    let n = (2.0 * half / minor_step).round() as i64;
    for i in 0..=n {
        let v = -half + i as f64 * minor_step;
        let is_major = (v / major_step).round() * major_step - v;
        let target = if is_major.abs() < minor_step * 1e-6 { &mut major } else { &mut minor };
        // Line parallel to Y at x=v, and parallel to X at y=v.
        target.extend_from_slice(&[v as f32, -half as f32, 0.0, v as f32, half as f32, 0.0]);
        target.extend_from_slice(&[-half as f32, v as f32, 0.0, half as f32, v as f32, 0.0]);
    }
    GridLines { minor, major, minor_step }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_covers_scene_and_is_bounded() {
        let g = build_grid(Some(([-3.0, -2.0, 0.0], [7.0, 4.0, 5.0])));
        assert!(!g.minor.is_empty() && !g.major.is_empty());
        let all: Vec<&f32> = g.minor.iter().chain(g.major.iter()).collect();
        let max = all.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(max >= 7.0, "grid must reach past the scene, got {max}");
        assert!(g.minor.len() + g.major.len() < 20_000, "grid stays modest");
    }

    #[test]
    fn default_grid_without_bounds() {
        let g = build_grid(None);
        assert!(!g.major.is_empty());
    }
}
