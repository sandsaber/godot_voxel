//! Integration test: run the transvoxel mesher on an SDF sphere and verify
//! the produced mesh is geometrically sensible.
//!
//! This is the first end-to-end validation of the Phase 0 port. A full
//! byte-for-byte parity test against the C++ golden output is added in a
//! later step (requires generating golden data from the C++ build).

use voxel_core::math::Vector3i;
use voxel_core::meshers::transvoxel::{
    build_regular_mesh, BuildRegularMeshParams, Cache, MeshArrays, RegularMesherInput,
};
use voxel_core::storage::{ChannelDepth, DenseVoxelBuffer, VoxelBufferRead};

/// An SDF sampled into a DenseVoxelBuffer, exposed through RegularMesherInput.
/// The mesher works on signed distances; we use a sphere of radius `r` centered
/// in the block so negative = inside.
struct SphereInput {
    buf: DenseVoxelBuffer,
}

impl SphereInput {
    /// Build a padded block of `inner`³ voxels containing an SDF sphere of the
    /// given radius, centered in the inner region.
    fn new(inner: i32, radius: f32) -> Self {
        // Padded block size: inner + MIN_PADDING + MAX_PADDING on every axis.
        let size = Vector3i::new(
            inner + MIN_PADDING + MAX_PADDING,
            inner + MIN_PADDING + MAX_PADDING,
            inner + MIN_PADDING + MAX_PADDING,
        );
        let mut buf = DenseVoxelBuffer::new(size, ChannelDepth::Bit32);

        // Sphere center in inner-block coordinates.
        let cx = (inner as f32) * 0.5;
        let cy = (inner as f32) * 0.5;
        let cz = (inner as f32) * 0.5;

        let sy = size.y as usize;
        let sx = size.x as usize;
        for z in 0..size.z as usize {
            for y in 0..size.y as usize {
                for x in 0..size.x as usize {
                    // Convert padded coords to inner coords by subtracting MIN_PADDING.
                    let ix = x as f32 - MIN_PADDING as f32;
                    let iy = y as f32 - MIN_PADDING as f32;
                    let iz = z as f32 - MIN_PADDING as f32;
                    // Signed distance to the sphere surface (positive outside).
                    let d =
                        ((ix - cx).powi(2) + (iy - cy).powi(2) + (iz - cz).powi(2)).sqrt() - radius;
                    // godot_voxel stores SDF so that POSITIVE values mean inside solid
                    // (the algorithm negates via sdf_as_float before comparison).
                    // So we store the negation of the geometric distance.
                    let stored = -d;
                    // ZXY layout: index = y + sy*(x + sx*z). Y innermost.
                    let i = y + sy * (x + sx * z);
                    let bytes = (stored).to_le_bytes();
                    buf.data_mut()[i * 4..i * 4 + 4].copy_from_slice(&bytes);
                }
            }
        }
        Self { buf }
    }
}

const MIN_PADDING: i32 = 1;
const MAX_PADDING: i32 = 2;

impl RegularMesherInput for SphereInput {
    fn len(&self) -> usize {
        (self.buf.size().x as usize) * (self.buf.size().y as usize) * (self.buf.size().z as usize)
    }
    fn block_size(&self) -> Vector3i {
        self.buf.size()
    }
    fn sample_f32(&self, data_index: usize) -> f32 {
        // Return the stored value directly; it is already in the algorithm's
        // convention (positive = inside solid, since we negated when storing).
        let b = self.buf.data();
        let off = data_index * 4;
        f32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }
}

#[test]
fn sphere_produces_a_closed_mesh() {
    let input = SphereInput::new(16, 6.0);
    let mut cache = Cache::default();
    let mut output = MeshArrays::default();
    let params = BuildRegularMeshParams {
        lod_index: 0,
        edge_clamp_margin: 0.0,
    };

    build_regular_mesh(&input, &params, &mut cache, &mut output);

    // A sphere of radius 6 in a 16³ block must produce a non-trivial mesh.
    assert!(
        output.vertices.len() > 100,
        "expected a substantial mesh, got {} vertices",
        output.vertices.len()
    );
    assert_eq!(output.normals.len(), output.vertices.len());
    assert_eq!(output.lod_data.len(), output.vertices.len());
    assert_eq!(output.indices.len() % 3, 0, "indices must be triangles");
    assert!(
        output.indices.len() > 300,
        "expected many triangles, got {} indices",
        output.indices.len()
    );

    // Every index must point at a valid vertex.
    let n = output.vertices.len() as i32;
    for &idx in &output.indices {
        assert!(idx >= 0 && idx < n, "index {} out of range [0,{})", idx, n);
    }

    // All vertices should lie within the inner block bounds (0..16).
    for v in &output.vertices {
        assert!(
            v.x >= -0.5 && v.x <= 16.5,
            "vertex x out of bounds: {}",
            v.x
        );
        assert!(
            v.y >= -0.5 && v.y <= 16.5,
            "vertex y out of bounds: {}",
            v.y
        );
        assert!(
            v.z >= -0.5 && v.z <= 16.5,
            "vertex z out of bounds: {}",
            v.z
        );
    }
}

#[test]
fn empty_block_produces_no_geometry() {
    // A block of all-positive SDF (fully outside) must yield no mesh.
    let input = SphereInput {
        buf: DenseVoxelBuffer::new(
            Vector3i::new(
                4 + MIN_PADDING + MAX_PADDING,
                4 + MIN_PADDING + MAX_PADDING,
                4 + MIN_PADDING + MAX_PADDING,
            ),
            ChannelDepth::Bit32,
        ),
    };
    let mut cache = Cache::default();
    let mut output = MeshArrays::default();
    build_regular_mesh(&input, &params_default(), &mut cache, &mut output);
    assert_eq!(output.vertices.len(), 0);
    assert_eq!(output.indices.len(), 0);
}

fn params_default() -> BuildRegularMeshParams {
    BuildRegularMeshParams {
        lod_index: 0,
        edge_clamp_margin: 0.0,
    }
}
