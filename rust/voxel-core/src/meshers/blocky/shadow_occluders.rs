//! Shadow occluder geometry generation for the blocky mesher.
//!
//! Ported from `meshers/blocky/blocky_shadow_occluders.{h,cpp}`. Produces a
//! single axis-aligned box (one quad per occluded chunk face) that the renderer
//! can use as a shadow occluder. The box covers the whole interior of the
//! chunk; whether each of the six faces is emitted is decided by classifying
//! the voxel buffer: a face is emitted only if *every* voxel pair straddling
//! that face fully occludes each other (both solid, opaque, with full sides).
//!
//! ## Differences from the C++ source
//! - The C++ entry point dispatches on a `VoxelBuffer::Depth` enum (8-bit vs
//!   16-bit) and reinterpret-casts the raw byte buffer. This port is generic
//!   over `T: Copy + Into<u16>` instead, mirroring [`crate::meshers::blocky::
//!   mesher::generate_mesh`]. Callers pass the typed slice directly.
//! - The C++ `OccluderArrays` is named `ShadowOccluderArrays` here for
//!   clarity and to match the module name.
//!
//! ## Bit-mask conventions (important — two different orderings!)
//! - `enabled_mask` (the caller-supplied parameter) uses the
//!   `VoxelMesherBlocky::Side` bit order: `NEGATIVE_X=0, POSITIVE_X=1,
//!   NEGATIVE_Y=2, POSITIVE_Y=3, NEGATIVE_Z=4, POSITIVE_Z=5`. This is the
//!   *opposite* X ordering from [`crate::constants::cube_tables::SideAxis`].
//! - `BakedModelMesh::full_sides_mask` uses [`crate::constants::cube_tables::
//!   SideAxis`] order: `POSITIVE_X=0, NEGATIVE_X=1, ...`. This matches
//!   `Cube::SideAxis` in the C++ source, which is what
//!   `classify_chunk_occlusion_from_voxels` indexes with.
//!
//! Both conventions are reproduced exactly from the C++ so the parity port
//! produces identical results.
//
// The `generate_occluders_geometry` body mirrors the C++ source line-for-line,
// including its `vi0 + 0` index expressions. Clippy's `identity_op` lint would
// rewrite those to bare `vi0`, which obscures the per-quad offset pattern when
// diffing against the C++; the lint is silenced for that reason.
#![allow(clippy::identity_op)]

use crate::constants::cube_tables::SideAxis;
use crate::math::{Vector3f, Vector3i};
use crate::meshers::blocky::baked_library::BakedLibrary;
use crate::meshers::blocky::mesher::PADDING;

/// `OccluderArrays` from the C++ header: a flat positions list + an index
/// buffer. Renamed `ShadowOccluderArrays` for clarity.
#[derive(Debug, Default, Clone)]
pub struct ShadowOccluderArrays {
    /// Vertex positions (3 floats per vertex, laid out as [`Vector3f`]).
    pub vertices: Vec<Vector3f>,
    /// Triangle indices into [`vertices`](Self::vertices).
    pub indices: Vec<i32>,
}

impl ShadowOccluderArrays {
    /// Create an empty occluder array.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the array holds any geometry.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Bit constants for the `enabled_mask` parameter.
//
// These use the `VoxelMesherBlocky::Side` ordering (NegativeX=0, PositiveX=1,
// ...), NOT the `SideAxis` ordering used by `full_sides_mask`. Reproduced
// verbatim from the C++ `enum Side` in `voxel_mesher_blocky.h`.
// ---------------------------------------------------------------------------

/// `enabled_mask` bit for the −X face.
pub const MASK_NEGATIVE_X: u8 = 1 << 0;
/// `enabled_mask` bit for the +X face.
pub const MASK_POSITIVE_X: u8 = 1 << 1;
/// `enabled_mask` bit for the −Y face.
pub const MASK_NEGATIVE_Y: u8 = 1 << 2;
/// `enabled_mask` bit for the +Y face.
pub const MASK_POSITIVE_Y: u8 = 1 << 3;
/// `enabled_mask` bit for the −Z face.
pub const MASK_NEGATIVE_Z: u8 = 1 << 4;
/// `enabled_mask` bit for the +Z face.
pub const MASK_POSITIVE_Z: u8 = 1 << 5;
/// All six faces enabled.
pub const MASK_ALL: u8 = MASK_NEGATIVE_X
    | MASK_POSITIVE_X
    | MASK_NEGATIVE_Y
    | MASK_POSITIVE_Y
    | MASK_NEGATIVE_Z
    | MASK_POSITIVE_Z;

// ---------------------------------------------------------------------------
// generate_occluders_geometry
// ---------------------------------------------------------------------------

/// `generate_occluders_geometry` — emit one quad (4 verts + 2 triangles) for
/// each requested face of the chunk-sized box `[0, maxf]`. The face set is
/// chosen by the six `bool` parameters; vertices are appended to `out_arrays`.
///
/// Ported verbatim from the C++ source, including the per-face winding orders
/// (which differ between faces: positive_x uses `0,1,2 / 0,2,3` while
/// negative_x uses `0,2,1 / 0,3,2`, etc.).
#[allow(clippy::too_many_arguments)]
pub fn generate_occluders_geometry(
    out_arrays: &mut ShadowOccluderArrays,
    maxf: Vector3f,
    positive_x: bool,
    positive_y: bool,
    positive_z: bool,
    negative_x: bool,
    negative_y: bool,
    negative_z: bool,
) {
    let quad_count = u32::from(positive_x)
        + u32::from(positive_y)
        + u32::from(positive_z)
        + u32::from(negative_x)
        + u32::from(negative_y)
        + u32::from(negative_z);

    if quad_count == 0 {
        return;
    }

    let vert_start = out_arrays.vertices.len();
    let idx_start = out_arrays.indices.len();
    out_arrays.vertices.resize(
        vert_start + 4 * quad_count as usize,
        Vector3f::new(0.0, 0.0, 0.0),
    );
    out_arrays
        .indices
        .resize(idx_start + 6 * quad_count as usize, 0);

    // `vi0`/`ii0` are absolute offsets into the (possibly non-empty) output
    // buffers; the C++ source uses 0-relative spans into the just-appended
    // region, which is equivalent to starting from `vert_start`/`idx_start`.
    let mut vi0 = vert_start as u32;
    let mut ii0 = idx_start as u32;

    macro_rules! put_vert {
        ($i:expr, $v:expr) => {
            out_arrays.vertices[(vi0 + $i) as usize] = $v;
        };
    }
    macro_rules! put_idx {
        ($i:expr, $v:expr) => {
            out_arrays.indices[(ii0 + $i) as usize] = ($v) as i32;
        };
    }

    if positive_x {
        // 3---2  y
        // |   |  |
        // 0---1  x--z
        put_vert!(0, Vector3f::new(maxf.x, 0.0, 0.0));
        put_vert!(1, Vector3f::new(maxf.x, 0.0, maxf.z));
        put_vert!(2, Vector3f::new(maxf.x, maxf.y, maxf.z));
        put_vert!(3, Vector3f::new(maxf.x, maxf.y, 0.0));

        put_idx!(0, vi0 + 0);
        put_idx!(1, vi0 + 1);
        put_idx!(2, vi0 + 2);
        put_idx!(3, vi0 + 0);
        put_idx!(4, vi0 + 2);
        put_idx!(5, vi0 + 3);

        vi0 += 4;
        ii0 += 6;
    }

    if positive_y {
        // 3---2  z
        // |   |  |
        // 0---1  y--x
        put_vert!(0, Vector3f::new(0.0, maxf.y, 0.0));
        put_vert!(1, Vector3f::new(maxf.x, maxf.y, 0.0));
        put_vert!(2, Vector3f::new(maxf.x, maxf.y, maxf.z));
        put_vert!(3, Vector3f::new(0.0, maxf.y, maxf.z));

        put_idx!(0, vi0 + 0);
        put_idx!(1, vi0 + 1);
        put_idx!(2, vi0 + 2);
        put_idx!(3, vi0 + 0);
        put_idx!(4, vi0 + 2);
        put_idx!(5, vi0 + 3);

        vi0 += 4;
        ii0 += 6;
    }

    if positive_z {
        // 3---2  y
        // |   |  |
        // 0---1  z--x
        put_vert!(0, Vector3f::new(0.0, 0.0, maxf.z));
        put_vert!(1, Vector3f::new(maxf.x, 0.0, maxf.z));
        put_vert!(2, Vector3f::new(maxf.x, maxf.y, maxf.z));
        put_vert!(3, Vector3f::new(0.0, maxf.y, maxf.z));

        // Note: swapped 1<->2 winding vs positive_x.
        put_idx!(0, vi0 + 0);
        put_idx!(1, vi0 + 2);
        put_idx!(2, vi0 + 1);
        put_idx!(3, vi0 + 0);
        put_idx!(4, vi0 + 3);
        put_idx!(5, vi0 + 2);

        vi0 += 4;
        ii0 += 6;
    }

    if negative_x {
        // 3---2  y
        // |   |  |
        // 0---1  x--z
        put_vert!(0, Vector3f::new(0.0, 0.0, 0.0));
        put_vert!(1, Vector3f::new(0.0, 0.0, maxf.z));
        put_vert!(2, Vector3f::new(0.0, maxf.y, maxf.z));
        put_vert!(3, Vector3f::new(0.0, maxf.y, 0.0));

        put_idx!(0, vi0 + 0);
        put_idx!(1, vi0 + 2);
        put_idx!(2, vi0 + 1);
        put_idx!(3, vi0 + 0);
        put_idx!(4, vi0 + 3);
        put_idx!(5, vi0 + 2);

        vi0 += 4;
        ii0 += 6;
    }

    if negative_y {
        // 3---2  z
        // |   |  |
        // 0---1  y--x
        put_vert!(0, Vector3f::new(0.0, 0.0, 0.0));
        put_vert!(1, Vector3f::new(maxf.x, 0.0, 0.0));
        put_vert!(2, Vector3f::new(maxf.x, 0.0, maxf.z));
        put_vert!(3, Vector3f::new(0.0, 0.0, maxf.z));

        put_idx!(0, vi0 + 0);
        put_idx!(1, vi0 + 2);
        put_idx!(2, vi0 + 1);
        put_idx!(3, vi0 + 0);
        put_idx!(4, vi0 + 3);
        put_idx!(5, vi0 + 2);

        vi0 += 4;
        ii0 += 6;
    }

    if negative_z {
        // 3---2  y
        // |   |  |
        // 0---1  z--x
        put_vert!(0, Vector3f::new(0.0, 0.0, 0.0));
        put_vert!(1, Vector3f::new(maxf.x, 0.0, 0.0));
        put_vert!(2, Vector3f::new(maxf.x, maxf.y, 0.0));
        put_vert!(3, Vector3f::new(0.0, maxf.y, 0.0));

        // C++: this final face does NOT advance vi0/ii0 (no subsequent face).
        put_idx!(0, vi0 + 0);
        put_idx!(1, vi0 + 1);
        put_idx!(2, vi0 + 2);
        put_idx!(3, vi0 + 0);
        put_idx!(4, vi0 + 2);
        put_idx!(5, vi0 + 3);
    }
}

// ---------------------------------------------------------------------------
// classify_chunk_occlusion_from_voxels
// ---------------------------------------------------------------------------

/// `is_fully_occluded` — whether two adjacent voxels `v0` and `v1` fully
/// occlude each other on the shared face (both solid, both opaque, both with
/// full sides on the relevant face). Matches the C++ `L::is_fully_occluded`.
///
/// `side0_mask`/`side1_mask` are single-bit masks in
/// [`SideAxis`] order, indexing `BakedModelMesh::full_sides_mask`.
#[inline]
fn is_fully_occluded(
    v0: u16,
    v1: u16,
    baked_data: &BakedLibrary,
    side0_mask: u8,
    side1_mask: u8,
) -> bool {
    if (v0 as usize) >= baked_data.models.len() || (v1 as usize) >= baked_data.models.len() {
        return false;
    }
    let b0 = &baked_data.models[v0 as usize];
    let b1 = &baked_data.models[v1 as usize];
    if b0.empty || b1.empty {
        return false;
    }
    if b0.transparency_index > 0 || b1.transparency_index > 0 {
        // Either side is transparent.
        return false;
    }
    if (b0.model.full_sides_mask & side0_mask) == 0 {
        return false;
    }
    if (b1.model.full_sides_mask & side1_mask) == 0 {
        return false;
    }
    true
}

/// Linear ZXY index for `(x, y, z)` in a buffer of size `block_size`. Matches
/// `Vector3iUtil::get_zxy_index(pos, block_size)`: `y + sy*(x + sx*z)`.
#[inline]
fn zxy_index(x: i32, y: i32, z: i32, block_size: Vector3i) -> usize {
    (y + block_size.y * (x + block_size.x * z)) as usize
}

/// `is_chunk_side_occluded_x` — true if every voxel pair straddling the chunk's
/// ±X boundary fully occludes each other. Matches the C++ helper of the same
/// name. `sign` is `+1` for the +X face, `-1` for the −X face.
fn is_chunk_side_occluded_x<T: Copy + Into<u16>>(
    min: Vector3i,
    max: Vector3i,
    id_buffer: &[T],
    block_size: Vector3i,
    sign: i32,
    baked_data: &BakedLibrary,
) -> bool {
    // Cube::SideAxis: PositiveX=0, NegativeX=1.
    let side0: SideAxis = if sign < 0 {
        SideAxis::NegativeX
    } else {
        SideAxis::PositiveX
    };
    let side1: SideAxis = if sign < 0 {
        SideAxis::PositiveX
    } else {
        SideAxis::NegativeX
    };
    let side0_mask = 1u8 << (side0 as u8);
    let side1_mask = 1u8 << (side1 as u8);

    let x0 = if sign < 0 { min.x } else { max.x - 1 };
    for z in min.z..max.z {
        for y in min.y..max.y {
            let loc0 = zxy_index(x0, y, z, block_size);
            let loc1 = zxy_index(x0 + sign, y, z, block_size);
            let v0: u16 = id_buffer[loc0].into();
            let v1: u16 = id_buffer[loc1].into();
            if !is_fully_occluded(v0, v1, baked_data, side0_mask, side1_mask) {
                return false;
            }
        }
    }
    true
}

/// `is_chunk_side_occluded_y` — true if every voxel pair straddling the chunk's
/// ±Y boundary fully occludes each other.
fn is_chunk_side_occluded_y<T: Copy + Into<u16>>(
    min: Vector3i,
    max: Vector3i,
    id_buffer: &[T],
    block_size: Vector3i,
    sign: i32,
    baked_data: &BakedLibrary,
) -> bool {
    // Cube::SideAxis: NegativeY=2, PositiveY=3.
    let side0: SideAxis = if sign < 0 {
        SideAxis::NegativeY
    } else {
        SideAxis::PositiveY
    };
    let side1: SideAxis = if sign < 0 {
        SideAxis::PositiveY
    } else {
        SideAxis::NegativeY
    };
    let side0_mask = 1u8 << (side0 as u8);
    let side1_mask = 1u8 << (side1 as u8);

    let y0 = if sign < 0 { min.y } else { max.y - 1 };
    for z in min.z..max.z {
        for x in min.x..max.x {
            let loc0 = zxy_index(x, y0, z, block_size);
            let loc1 = zxy_index(x, y0 + sign, z, block_size);
            let v0: u16 = id_buffer[loc0].into();
            let v1: u16 = id_buffer[loc1].into();
            if !is_fully_occluded(v0, v1, baked_data, side0_mask, side1_mask) {
                return false;
            }
        }
    }
    true
}

/// `is_chunk_side_occluded_z` — true if every voxel pair straddling the chunk's
/// ±Z boundary fully occludes each other.
fn is_chunk_side_occluded_z<T: Copy + Into<u16>>(
    min: Vector3i,
    max: Vector3i,
    id_buffer: &[T],
    block_size: Vector3i,
    sign: i32,
    baked_data: &BakedLibrary,
) -> bool {
    // Cube::SideAxis: NegativeZ=4, PositiveZ=5.
    let side0: SideAxis = if sign < 0 {
        SideAxis::NegativeZ
    } else {
        SideAxis::PositiveZ
    };
    let side1: SideAxis = if sign < 0 {
        SideAxis::PositiveZ
    } else {
        SideAxis::NegativeZ
    };
    let side0_mask = 1u8 << (side0 as u8);
    let side1_mask = 1u8 << (side1 as u8);

    let z0 = if sign < 0 { min.z } else { max.z - 1 };
    for x in min.x..max.x {
        for y in min.y..max.y {
            let loc0 = zxy_index(x, y, z0, block_size);
            let loc1 = zxy_index(x, y, z0 + sign, block_size);
            let v0: u16 = id_buffer[loc0].into();
            let v1: u16 = id_buffer[loc1].into();
            if !is_fully_occluded(v0, v1, baked_data, side0_mask, side1_mask) {
                return false;
            }
        }
    }
    true
}

/// Per-face occlusion classification. Ported from the C++
/// `classify_chunk_occlusion_from_voxels`. For each of the six faces, returns
/// whether that face of the chunk box should be emitted as an occluder.
///
/// `enabled_mask` uses the [`MASK_*`](self) bit constants
/// (`VoxelMesherBlocky::Side` order: NegativeX=0, PositiveX=1, ...). A face is
/// emitted only if it is both enabled *and* fully occluded by the voxel pairs
/// straddling that boundary.
pub fn classify_chunk_occlusion_from_voxels<T: Copy + Into<u16>>(
    id_buffer: &[T],
    baked_data: &BakedLibrary,
    block_size: Vector3i,
    min: Vector3i,
    max: Vector3i,
    enabled_mask: u8,
) -> (bool, bool, bool, bool, bool, bool) {
    // enabled_mask uses VoxelMesherBlocky::Side order.
    let positive_x_enabled = (enabled_mask & MASK_POSITIVE_X) != 0;
    let positive_y_enabled = (enabled_mask & MASK_POSITIVE_Y) != 0;
    let positive_z_enabled = (enabled_mask & MASK_POSITIVE_Z) != 0;
    let negative_x_enabled = (enabled_mask & MASK_NEGATIVE_X) != 0;
    let negative_y_enabled = (enabled_mask & MASK_NEGATIVE_Y) != 0;
    let negative_z_enabled = (enabled_mask & MASK_NEGATIVE_Z) != 0;

    let positive_x = positive_x_enabled
        && is_chunk_side_occluded_x(min, max, id_buffer, block_size, 1, baked_data);
    let positive_y = positive_y_enabled
        && is_chunk_side_occluded_y(min, max, id_buffer, block_size, 1, baked_data);
    let positive_z = positive_z_enabled
        && is_chunk_side_occluded_z(min, max, id_buffer, block_size, 1, baked_data);

    let negative_x = negative_x_enabled
        && is_chunk_side_occluded_x(min, max, id_buffer, block_size, -1, baked_data);
    let negative_y = negative_y_enabled
        && is_chunk_side_occluded_y(min, max, id_buffer, block_size, -1, baked_data);
    let negative_z = negative_z_enabled
        && is_chunk_side_occluded_z(min, max, id_buffer, block_size, -1, baked_data);

    (
        positive_x, positive_y, positive_z, negative_x, negative_y, negative_z,
    )
}

// ---------------------------------------------------------------------------
// generate_shadow_occluders (entry point)
// ---------------------------------------------------------------------------

/// `generate_shadow_occluders` — main entry point. Classifies which chunk
/// faces are fully occluded and emits one quad per occluded (and enabled) face
/// into `out_arrays`. The box covers the chunk interior `[0, max-min]`.
///
/// Generic over the voxel-id type `T` (the C++ version dispatches on
/// `VoxelBuffer::Depth` and reinterpret-casts the byte buffer; callers here
/// pass the typed slice directly).
///
/// * `id_buffer` — flat voxel-id channel, ZXY layout, padded by [`PADDING`].
/// * `block_size` — full (padded) block dimensions.
/// * `enabled_mask` — bitmask of which faces are allowed to be emitted
///   (see the [`MASK_*`](self) constants). Use [`MASK_ALL`] to allow all six.
pub fn generate_shadow_occluders<T: Copy + Into<u16>>(
    out_arrays: &mut ShadowOccluderArrays,
    id_buffer: &[T],
    block_size: Vector3i,
    baked_data: &BakedLibrary,
    enabled_mask: u8,
) {
    // Data must be padded, hence the off-by-one.
    let min = Vector3i::splat(PADDING);
    let max = block_size - Vector3i::splat(PADDING);

    let (positive_x, positive_y, positive_z, negative_x, negative_y, negative_z) =
        classify_chunk_occlusion_from_voxels(
            id_buffer,
            baked_data,
            block_size,
            min,
            max,
            enabled_mask,
        );

    // `to_vec3f(max - min)` is the chunk interior size.
    let maxf = Vector3f::new(
        (max.x - min.x) as f32,
        (max.y - min.y) as f32,
        (max.z - min.z) as f32,
    );

    generate_occluders_geometry(
        out_arrays, maxf, positive_x, positive_y, positive_z, negative_x, negative_y, negative_z,
    );
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::cube_tables::{Side, CORNER_POSITION, SIDE_CORNERS, SIDE_QUAD_TRIANGLES};
    use crate::math::Vector2f;
    use crate::meshers::blocky::baked_library::{BakedLibrary, BakedModel};
    use crate::meshers::blocky::SideSurface;

    /// Build a full unit-cube side surface for `side` (4 corners + 2 tris).
    fn full_cube_side_surface(side: usize) -> SideSurface {
        let corners = SIDE_CORNERS[side];
        let positions: Vec<Vector3f> = corners.iter().map(|&c| CORNER_POSITION[c]).collect();
        let indices: Vec<i32> = SIDE_QUAD_TRIANGLES[side].to_vec();
        let uvs: Vec<Vector2f> = vec![
            Vector2f::new(0.0, 0.0),
            Vector2f::new(1.0, 0.0),
            Vector2f::new(1.0, 1.0),
            Vector2f::new(0.0, 1.0),
        ];
        SideSurface {
            positions,
            uvs,
            indices,
            tangents: Vec::new(),
        }
    }

    /// A baked library whose model 1 is a full opaque cube (all six faces
    /// full). Mirrors `mesher::tests::full_cube_library_baked`.
    fn full_cube_library_baked() -> BakedLibrary {
        let air = BakedModel::default();
        let mut cube = BakedModel {
            empty: false,
            culls_neighbors: true,
            contributes_to_ao: true,
            ..Default::default()
        };
        cube.model.surface_count = 1;
        cube.model.surfaces[0].material_id = 0;
        cube.model.surfaces[0].collision_enabled = true;
        for side in 0..Side::COUNT {
            cube.model.sides_surfaces[side][0] = full_cube_side_surface(side);
        }
        let mut lib = BakedLibrary {
            models: vec![air, cube],
            indexed_materials_count: 1,
            ..Default::default()
        };
        crate::meshers::blocky::bake_library(&mut lib);
        lib
    }

    #[test]
    fn occluder_arrays_default_is_empty() {
        let a = ShadowOccluderArrays::new();
        assert!(a.is_empty());
        assert_eq!(a.vertices.len(), 0);
        assert_eq!(a.indices.len(), 0);
    }

    #[test]
    fn generate_occluders_geometry_no_faces_emits_nothing() {
        let mut out = ShadowOccluderArrays::new();
        generate_occluders_geometry(
            &mut out,
            Vector3f::new(1.0, 1.0, 1.0),
            false,
            false,
            false,
            false,
            false,
            false,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn generate_occluders_geometry_all_faces_emits_six_quads() {
        let mut out = ShadowOccluderArrays::new();
        generate_occluders_geometry(
            &mut out,
            Vector3f::new(2.0, 3.0, 4.0),
            true,
            true,
            true,
            true,
            true,
            true,
        );
        // 6 quads × 4 verts = 24 verts.
        assert_eq!(out.vertices.len(), 24);
        // 6 quads × 6 indices = 36 indices (12 triangles).
        assert_eq!(out.indices.len(), 36);
        assert_eq!(out.indices.len() / 3, 12);

        // All indices must point inside the vertex buffer.
        for &i in &out.indices {
            assert!((i as usize) < out.vertices.len(), "index out of range");
        }
    }

    #[test]
    fn generate_occluders_geometry_positive_x_face_corners() {
        let mut out = ShadowOccluderArrays::new();
        let maxf = Vector3f::new(2.0, 3.0, 4.0);
        // Only +X.
        generate_occluders_geometry(&mut out, maxf, true, false, false, false, false, false);
        assert_eq!(out.vertices.len(), 4);
        // The +X face sits at x = maxf.x for all four corners.
        for v in &out.vertices {
            assert_eq!(v.x, maxf.x);
        }
        // Y/Z span [0, maxf].
        assert_eq!(out.vertices[0], Vector3f::new(maxf.x, 0.0, 0.0));
        assert_eq!(out.vertices[1], Vector3f::new(maxf.x, 0.0, maxf.z));
        assert_eq!(out.vertices[2], Vector3f::new(maxf.x, maxf.y, maxf.z));
        assert_eq!(out.vertices[3], Vector3f::new(maxf.x, maxf.y, 0.0));
        // Winding for +X is 0,1,2 / 0,2,3.
        assert_eq!(&out.indices, &[0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn generate_occluders_geometry_appends_to_existing_buffer() {
        // Two calls should append, not overwrite.
        let mut out = ShadowOccluderArrays::new();
        generate_occluders_geometry(
            &mut out,
            Vector3f::new(1.0, 1.0, 1.0),
            true,
            false,
            false,
            false,
            false,
            false,
        );
        assert_eq!(out.vertices.len(), 4);
        let first_indices = out.indices.clone();

        generate_occluders_geometry(
            &mut out,
            Vector3f::new(1.0, 1.0, 1.0),
            false,
            false,
            false,
            false,
            false,
            true,
        );
        // 8 verts, 12 indices now.
        assert_eq!(out.vertices.len(), 8);
        assert_eq!(out.indices.len(), 12);
        // First quad's indices unchanged.
        assert_eq!(&out.indices[0..6], &first_indices[..]);
        // Second quad's indices start at vertex 4.
        assert_eq!(&out.indices[6..12], &[4, 5, 6, 4, 6, 7]);
    }

    /// A solid chunk of cube (every voxel = 1) with full padding: every face
    /// is fully occluded, so with `MASK_ALL` all six occluder quads are
    /// emitted.
    #[test]
    fn generate_shadow_occluders_solid_chunk_emits_all_faces() {
        // 4x4x4 block (1 padding each side → 2x2x2 interior).
        let size = Vector3i::new(4, 4, 4);
        let buf = vec![1u16; (size.x * size.y * size.z) as usize];
        let lib = full_cube_library_baked();

        let mut out = ShadowOccluderArrays::new();
        generate_shadow_occluders(&mut out, &buf, size, &lib, MASK_ALL);

        // All six faces are occluded → 6 quads.
        assert_eq!(out.vertices.len(), 24);
        assert_eq!(out.indices.len(), 36);
    }

    /// A chunk that is entirely air produces no occluders (the boundary voxel
    /// pairs are never fully occluded).
    #[test]
    fn generate_shadow_occluders_air_chunk_emits_nothing() {
        let size = Vector3i::new(4, 4, 4);
        let buf = vec![0u16; (size.x * size.y * size.z) as usize];
        let lib = full_cube_library_baked();

        let mut out = ShadowOccluderArrays::new();
        generate_shadow_occluders(&mut out, &buf, size, &lib, MASK_ALL);
        assert!(out.is_empty());
    }

    /// `enabled_mask` gates which faces may be emitted even when occluded.
    #[test]
    fn generate_shadow_occluders_respects_enabled_mask() {
        let size = Vector3i::new(4, 4, 4);
        let buf = vec![1u16; (size.x * size.y * size.z) as usize];
        let lib = full_cube_library_baked();

        // Only +X allowed.
        let mut out = ShadowOccluderArrays::new();
        generate_shadow_occluders(&mut out, &buf, size, &lib, MASK_POSITIVE_X);
        assert_eq!(out.vertices.len(), 4, "only the +X face should be emitted");
        assert_eq!(out.indices.len(), 6);

        // No faces allowed.
        let mut out2 = ShadowOccluderArrays::new();
        generate_shadow_occluders(&mut out2, &buf, size, &lib, 0);
        assert!(out2.is_empty());
    }

    /// If only one boundary voxel pair is air, that face is no longer fully
    /// occluded and is skipped. Verifies the "every pair must occlude" rule.
    #[test]
    fn classify_skips_face_when_one_pair_is_air() {
        let size = Vector3i::new(4, 4, 4);
        let mut buf = vec![1u16; (size.x * size.y * size.z) as usize];
        let lib = full_cube_library_baked();

        // Punch a hole: make the +X boundary voxel at one corner air on the
        // interior side, so the (interior, padding) pair for +X is not fully
        // occluded there. Interior max.x - 1 = 4 - 1 - 1 = 2; padding neighbor
        // at x=3.
        let row = size.y;
        let deck = size.x * row;
        let idx = |x: i32, y: i32, z: i32| (y + x * row + z * deck) as usize;
        buf[idx(2, 1, 1)] = 0; // interior voxel adjacent to the +X padding

        let (px, _py, _pz, _nx, _ny, _nz) = classify_chunk_occlusion_from_voxels(
            &buf,
            &lib,
            size,
            Vector3i::splat(PADDING),
            size - Vector3i::splat(PADDING),
            MASK_ALL,
        );
        assert!(!px, "+X face must not be occluded when one pair is air");
    }

    /// `is_fully_occluded` rejects transparent neighbors.
    #[test]
    fn is_fully_occluded_rejects_transparent() {
        let lib = full_cube_library_baked();
        // Make a transparent copy of the cube by checking the rule with a
        // model that has transparency_index > 0. We synthesize a tiny library
        // inline rather than mutating the baked one (which would invalidate
        // the culling matrix).
        let mut lib2 = lib.clone();
        lib2.models.push(BakedModel {
            empty: false,
            transparency_index: 1,
            ..lib2.models[1].clone()
        });
        let cube = 1u16;
        let transparent = 2u16;
        // Two opaque cubes fully occlude.
        assert!(is_fully_occluded(
            cube,
            cube,
            &lib,
            1 << SideAxis::PositiveX as u8,
            1 << SideAxis::NegativeX as u8
        ));
        // Cube vs transparent does NOT occlude.
        assert!(!is_fully_occluded(
            cube,
            transparent,
            &lib2,
            1 << SideAxis::PositiveX as u8,
            1 << SideAxis::NegativeX as u8
        ));
    }

    /// Sanity-check the two different bit-order conventions don't get mixed:
    /// `MASK_POSITIVE_X` (VoxelMesherBlocky::Side order) must be bit 1, while
    /// `SideAxis::PositiveX` (full_sides_mask order) must be bit 0.
    #[test]
    fn mask_constants_use_correct_bit_orders() {
        // enabled_mask uses VoxelMesherBlocky::Side: NegX=0, PosX=1, ...
        assert_eq!(MASK_NEGATIVE_X, 1 << 0);
        assert_eq!(MASK_POSITIVE_X, 1 << 1);
        assert_eq!(MASK_NEGATIVE_Y, 1 << 2);
        assert_eq!(MASK_POSITIVE_Y, 1 << 3);
        assert_eq!(MASK_NEGATIVE_Z, 1 << 4);
        assert_eq!(MASK_POSITIVE_Z, 1 << 5);
        assert_eq!(MASK_ALL, 0b111111);

        // full_sides_mask uses SideAxis: PosX=0, NegX=1, ...
        assert_eq!(1u8 << SideAxis::PositiveX as u8, 1 << 0);
        assert_eq!(1u8 << SideAxis::NegativeX as u8, 1 << 1);
        assert_eq!(1u8 << SideAxis::PositiveZ as u8, 1 << 5);
    }
}
