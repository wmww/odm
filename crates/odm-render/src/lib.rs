//! wgpu renderer for ODM IR scenes: one render path serving both offscreen
//! PNG (headless/CLI) and the viewer viewport.
//!
//! Conventions: Z-up, right-handed, meshes are indexed CCW triangles with
//! positions only — flat normals come from screen-space derivatives in the
//! fragment shader.

mod camera;
mod flatten;
mod gpu;
mod grid;
pub mod math;
mod wire;

pub use camera::{Camera, Projection};
pub use flatten::{DEFAULT_COLOR, FlatInstance, flatten_node, flatten_scene, mesh_aabb, node_id};
pub use gpu::{COLOR_FORMAT, DEPTH_FORMAT, MSAA_SAMPLES, Renderer};
pub use wire::{WireHit, mesh_edges, pick_wire};

/// Re-exported so the viewer uses the exact same wgpu version.
pub use wgpu;

use odm_ir::Hash;
use std::collections::HashMap;
use std::sync::Arc;

/// Wireframe line thickness in pixels. Not a UI setting — change it here.
/// Wires are screen-space quads, so this is exact at any zoom (WebGPU line
/// primitives are always 1px).
pub const WIRE_WIDTH_PX: f32 = 2.0;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no compatible GPU adapter: {0}")]
    NoAdapter(String),
    #[error("GPU device request failed: {0}")]
    Device(String),
    #[error("object {0} missing from store")]
    MissingObject(Hash),
    #[error("invalid scene: {0}")]
    BadScene(String),
    #[error("invalid render options: {0}")]
    BadOptions(String),
    #[error("GPU error: {0}")]
    Gpu(String),
    #[error("PNG encoding failed: {0}")]
    Png(String),
}

pub struct RenderInstance {
    pub mesh: Hash,
    /// Column-major world matrix.
    pub transform: [[f32; 4]; 4],
    /// Linear RGBA, already resolved through inheritance.
    pub color: [f32; 4],
}

pub struct RenderScene {
    pub instances: Vec<RenderInstance>,
    pub meshes: HashMap<Hash, Arc<odm_ir::Mesh>>,
    /// World AABB of all instances.
    pub bounds: Option<([f64; 3], [f64; 3])>,
}

pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
    pub camera: Camera,
    /// Draw mesh edges only, in each instance's own color — no solid surfaces,
    /// so everything behind shows through.
    pub wireframe: bool,
    /// Ground grid on the z=0 plane.
    pub grid: bool,
    /// Linear RGBA clear color.
    pub background: [f32; 4],
}

impl RenderOptions {
    pub fn default_with(width: u32, height: u32) -> Self {
        RenderOptions {
            width,
            height,
            camera: Camera::Auto { direction: Camera::DEFAULT_DIR, ortho: false },
            wireframe: false,
            grid: true,
            background: [0.055, 0.058, 0.065, 1.0],
        }
    }
}
