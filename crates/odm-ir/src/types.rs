use crate::canon::{Canonical, Hasher, tag};
use crate::hash::Hash;
use serde::{Deserialize, Serialize};

/// Triangle mesh, indexed. Positions are xyz triples; indices are CCW
/// triangles. No stored normals — the renderer derives flat normals in the
/// fragment shader.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<f32>,
    pub indices: Vec<u32>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MeshError {
    #[error("positions length {0} is not a multiple of 3")]
    PositionsLen(usize),
    #[error("indices length {0} is not a multiple of 3")]
    IndicesLen(usize),
    #[error("index {index} out of range for {verts} vertices")]
    IndexRange { index: u32, verts: usize },
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn validate(&self) -> Result<(), MeshError> {
        if !self.positions.len().is_multiple_of(3) {
            return Err(MeshError::PositionsLen(self.positions.len()));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(MeshError::IndicesLen(self.indices.len()));
        }
        let verts = self.vertex_count();
        for &i in &self.indices {
            if i as usize >= verts {
                return Err(MeshError::IndexRange { index: i, verts });
            }
        }
        Ok(())
    }
}

impl Canonical for Mesh {
    fn write(&self, w: &mut Hasher) {
        w.u8(tag::MESH);
        w.f32s(&self.positions);
        w.u32s(&self.indices);
    }
}

/// Linear RGBA color.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
}

impl Canonical for Color {
    fn write(&self, w: &mut Hasher) {
        w.u8(tag::COLOR);
        w.f32(self.r);
        w.f32(self.g);
        w.f32(self.b);
        w.f32(self.a);
    }
}

/// 4x4 transform, column-major (three.js `Matrix4.elements` convention).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform(pub [f64; 16]);

impl Transform {
    pub const IDENTITY: Transform = Transform([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Canonical for Transform {
    fn write(&self, w: &mut Hasher) {
        w.u8(tag::TRANSFORM);
        for &v in &self.0 {
            w.f64(v);
        }
    }
}

/// Scene tree node. Children and meshes are both content hashes into the
/// store, so a shared subtree is stored (and hashed) once.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Node {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<Hash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Hash>,
}

impl Node {
    /// Store objects this node references directly: its mesh and its children.
    pub fn refs(&self, out: &mut Vec<Hash>) {
        out.extend(self.mesh);
        out.extend(self.children.iter().copied());
    }
}

impl Canonical for Node {
    fn write(&self, w: &mut Hasher) {
        w.u8(tag::NODE);
        match &self.name {
            None => w.u8(0),
            Some(s) => {
                w.u8(1);
                w.str(s);
            }
        }
        self.transform.write(w);
        w.opt(&self.color);
        w.opt(&self.mesh);
        w.len(self.children.len());
        for c in &self.children {
            w.hash(c);
        }
    }
}

