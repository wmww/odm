//! f32-regression tripwires for the render-side *reads* of mesh positions
//! (`mesh_aabb`, wire projection/picking). The GPU vertex upload is the one
//! place f32 belongs; everything else must stay f64. Probe: coordinates near
//! 1e7, where the f32 ulp is ~1.0 and sub-unit geometry does not survive.

use odm_ir::{Canonical, Mesh};
use odm_render::{Camera, Instance, RenderOptions, RenderScene, math, mesh_aabb, pick_wire};
use std::collections::HashMap;
use std::sync::Arc;

/// One triangle with sub-unit detail, x-offset by `off`.
fn tri(off: f64) -> Mesh {
    Mesh {
        positions: vec![off + 0.1, 0.0, 0.0, off + 0.9, 0.4, 0.0, off + 0.1, 0.8, 0.0],
        indices: vec![0, 1, 2],
    }
}

#[test]
fn mesh_aabb_is_exact_far_from_origin() {
    let m = tri(1e7);
    let (min, max) = mesh_aabb(&m).unwrap();
    // Bit equality: any f32 pass rounds these to whole units.
    assert_eq!(min[0], 1e7 + 0.1);
    assert_eq!(max[0], 1e7 + 0.9);
    assert_eq!(max[1], 0.8);
}

/// The same triangle picked through the same (relative) camera must project
/// identically at the origin and 1e7 away — wire math is f64 clip-space all
/// the way, so only position quantization could make them differ.
#[test]
fn wire_projection_matches_near_and_far() {
    let scene_at = |off: f64| {
        let mesh = tri(off);
        let hash = mesh.hash();
        RenderScene {
            instances: vec![Instance {
                id: String::new(),
                name: None,
                mesh: hash,
                world: math::IDENTITY,
                color: [1.0; 4],
            }],
            meshes: HashMap::from([(hash, Arc::new(mesh))]),
            bounds: mesh_aabb(&tri(off)),
        }
    };
    let opts_at = |off: f64| {
        let mut opts = RenderOptions::default_with(200, 200);
        opts.camera = Camera {
            eye: Some([off + 0.5, 0.4, 10.0]),
            target: Some([off + 0.5, 0.4, 0.0]),
            up: Some([0.0, 1.0, 0.0]),
            ortho: true,
            ortho_height: Some(2.0),
            ..Camera::default()
        };
        opts
    };
    // A pixel a few px off the A→B edge (midpoint (0.5, 0.2) → px (100, 120)).
    let point = [103.0, 122.0];
    let near = pick_wire(&scene_at(0.0), &opts_at(0.0), point, 8.0).expect("near pick hits");
    let far_off = 1e7 + 0.25;
    let far = pick_wire(&scene_at(far_off), &opts_at(far_off), point, 8.0).expect("far pick hits");
    assert!(
        (near.distance_px - far.distance_px).abs() < 1e-3,
        "screen-space drift far from origin: near {} px vs far {} px",
        near.distance_px,
        far.distance_px
    );
    assert!((near.depth - far.depth).abs() < 1e-6);
}
