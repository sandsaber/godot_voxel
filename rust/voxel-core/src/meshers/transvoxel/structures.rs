//! Transvoxel data structures.
//!
//! Ported from `meshers/transvoxel/transvoxel.h`:
//! - `MeshArrays` + `LodAttrib` (the mesher output)
//! - `ReuseCell` (vertex-reuse cache entry)
//! - `Cache` (two-deck reuse cache)

use crate::math::Vector3f;

/// Per-vertex LOD data, matching C++ `LodAttrib`. `#[repr(C)]` so the whole
/// mesh can be uploaded to the GPU as a struct-of-arrays buffer.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct LodAttrib {
    /// Secondary position used for LOD transition meshes.
    pub secondary_position: Vector3f,
    /// Mask telling if a cell the vertex belongs to is on a side of the block.
    pub cell_border_mask: u8,
    /// Mask telling if the vertex is on a side of the block.
    pub vertex_border_mask: u8,
    /// Flag telling if the vertex belongs to a transition mesh.
    pub transition: u8,
    /// Unused; aligns the struct to 4*sizeof(float).
    pub _pad: u8,
}

impl LodAttrib {
    pub fn new(
        secondary: Vector3f,
        cell_border_mask: u8,
        vertex_border_mask: u8,
        transition: u8,
    ) -> Self {
        Self {
            secondary_position: secondary,
            cell_border_mask,
            vertex_border_mask,
            transition,
            _pad: 0,
        }
    }
}

/// Output of the transvoxel mesher: parallel arrays for vertices, normals,
/// LOD data, and indices. Mirrors C++ `MeshArrays`.
#[derive(Debug, Default)]
pub struct MeshArrays {
    pub vertices: Vec<Vector3f>,
    pub normals: Vec<Vector3f>,
    pub lod_data: Vec<LodAttrib>,
    pub indices: Vec<i32>,
    // NOTE: texturing_data_{1f32,2f32} from C++ are omitted in the Phase 0 port
    // because the pilot only exercises TEXTURES_NONE. They reappear when the
    // material processors (single_s4 / mixel4) are ported.
}

impl MeshArrays {
    /// Append a vertex with all attributes; returns its index.
    ///
    /// Matches `MeshArrays::add_vertex` in transvoxel.h.
    pub fn add_vertex(
        &mut self,
        primary: Vector3f,
        normal: Vector3f,
        cell_border_mask: u8,
        vertex_border_mask: u8,
        transition: u8,
        secondary: Vector3f,
    ) -> i32 {
        let vi = self.vertices.len() as i32;
        self.vertices.push(primary);
        self.normals.push(normal);
        self.lod_data.push(LodAttrib::new(
            secondary,
            cell_border_mask,
            vertex_border_mask,
            transition,
        ));
        vi
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.normals.clear();
        self.lod_data.clear();
        self.indices.clear();
    }
}

/// A 4-entry vertex-reuse slot for regular cells. Matches C++ `ReuseCell`.
#[derive(Debug, Clone, Copy)]
pub struct ReuseCell {
    /// Vertex indices into the output mesh; -1 = unused.
    pub vertices: [i32; 4],
    /// Packed texture indices (used by material processors; 0 for TEXTURES_NONE).
    pub packed_texture_indices: u32,
}

impl Default for ReuseCell {
    fn default() -> Self {
        Self {
            vertices: [-1; 4],
            packed_texture_indices: 0,
        }
    }
}

/// Two-deck reuse cache for regular cells.
///
/// In C++ this is `class Cache` with `FixedArray<StdVector<ReuseCell>, 2>`.
/// We keep the same layout: two "decks" indexed by `pos.z & 1`, each holding
/// `block_size.x * block_size.y` cells.
#[derive(Debug)]
pub struct Cache {
    /// Two decks, indexed by `pos.z & 1`.
    decks: [Vec<ReuseCell>; 2],
    block_size: crate::math::Vector3i,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            decks: [Vec::new(), Vec::new()],
            block_size: crate::math::Vector3i::zero(),
        }
    }
}

impl Cache {
    /// Resize the reuse cache for a block of size `block_size_with_padding`.
    /// All entries are reset to "unused" (-1). Matches `reset_reuse_cells`.
    pub fn reset_reuse_cells(&mut self, block_size: crate::math::Vector3i) {
        self.block_size = block_size;
        let deck_area = (block_size.x as usize) * (block_size.y as usize);
        for deck in &mut self.decks {
            deck.clear();
            deck.resize(deck_area, ReuseCell::default());
        }
    }

    /// Fetch a reuse cell by position. Matches `get_reuse_cell(pos)`.
    #[inline]
    pub fn get_reuse_cell(&self, pos: crate::math::Vector3i) -> &ReuseCell {
        let j = (pos.z as usize) & 1;
        let i = (pos.y as usize) * (self.block_size.x as usize) + (pos.x as usize);
        &self.decks[j][i]
    }

    #[inline]
    pub fn get_reuse_cell_mut(&mut self, pos: crate::math::Vector3i) -> &mut ReuseCell {
        let j = (pos.z as usize) & 1;
        let i = (pos.y as usize) * (self.block_size.x as usize) + (pos.x as usize);
        &mut self.decks[j][i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vector3i;

    #[test]
    fn cache_reset_and_get() {
        let mut c = Cache::default();
        c.reset_reuse_cells(Vector3i::new(4, 4, 4));
        let cell = c.get_reuse_cell(Vector3i::new(0, 0, 0));
        assert_eq!(cell.vertices, [-1, -1, -1, -1]);
    }

    #[test]
    fn cache_deck_indexing() {
        // pos.z & 1 selects deck; this is the key invariant of the reuse cache.
        let mut c = Cache::default();
        c.reset_reuse_cells(Vector3i::new(2, 2, 4));
        // Write to z=0 deck
        c.get_reuse_cell_mut(Vector3i::new(0, 0, 0)).vertices[0] = 42;
        // z=2 is the same deck (2 & 1 == 0) but different cell (same x,y → same cell here)
        // Actually same x,y,z&1 → same cell. So writing z=0 then reading z=2 gives 42.
        assert_eq!(c.get_reuse_cell(Vector3i::new(0, 0, 2)).vertices[0], 42);
        // z=1 is the other deck → unmodified.
        assert_eq!(c.get_reuse_cell(Vector3i::new(0, 0, 1)).vertices[0], -1);
    }

    #[test]
    fn mesh_arrays_add_vertex_returns_indices() {
        let mut m = MeshArrays::default();
        let v0 = m.add_vertex(
            Vector3f::new(1.0, 0.0, 0.0),
            Vector3f::new(0.0, 1.0, 0.0),
            0,
            0,
            0,
            Vector3f::zero(),
        );
        let v1 = m.add_vertex(
            Vector3f::new(2.0, 0.0, 0.0),
            Vector3f::new(0.0, 1.0, 0.0),
            0,
            0,
            0,
            Vector3f::zero(),
        );
        assert_eq!(v0, 0);
        assert_eq!(v1, 1);
        assert_eq!(m.vertices.len(), 2);
        assert_eq!(m.normals.len(), 2);
        assert_eq!(m.lod_data.len(), 2);
    }
}
