//! Output arrays for the cubes mesher.
//!
//! Parallel `Vec`s of vertex attributes + an index buffer. The transvoxel
//! mesher's `MeshArrays` carries `LodAttrib` and no colors/UVs; the cubes
//! mesher needs colors but not LOD attribution, so it gets its own struct.

use crate::math::{Color, Vector2f, Vector3f};

/// Triangle-mesh output of the cubes mesher. All arrays are parallel: vertex
/// `i`'s position is `positions[i]`, its color is `colors[i]`, etc.
#[derive(Debug, Default, Clone)]
pub struct CubesArrays {
    pub positions: Vec<Vector3f>,
    pub normals: Vec<Vector3f>,
    pub colors: Vec<Color>,
    pub uvs: Vec<Vector2f>,
    pub indices: Vec<i32>,
}

impl CubesArrays {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.colors.clear();
        self.uvs.clear();
        self.indices.clear();
    }

    /// Total vertex count (inferred from positions).
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Total triangle count.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}
