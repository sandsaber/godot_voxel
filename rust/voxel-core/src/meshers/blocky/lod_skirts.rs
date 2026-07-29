//! LOD seam-skirt appending for the blocky mesher.
//!
//! Ported from `meshers/blocky/blocky_lod_skirts.h` (header-only templates).
//!
//! Adds extra side geometry on the chunk border for every border voxel exposed
//! to air, dropping the vertices down (or out) by `skirt_depth` to hide the
//! cracks that appear when meshes of different LOD are placed next to each
//! other. This method does not need access to the child LOD's voxels; the
//! trade-off is that it won't always hide every crack, but it hides most of
//! them. AO is not handled (and probably doesn't need to be).
//!
//! ## Differences from the C++ source
//! - The C++ `append_skirts` uses a `TintSampler` callback to modulate vertex
//!   colors (`voxel_baked_data.color * tint_sampler.evaluate(x, y, z)`). The
//!   tint sampler is wired to `VoxelBuffer` channels which are not yet ported,
//!   so this port takes `skirt_depth` instead and applies no per-vertex tint:
//!   the color is the model's `color` multiplied by the (uniform) skirt drop.
//!   When the `VoxelBuffer`/tint layer is ported (Phase 5) the sampler can be
//!   threaded through here without changing the algorithm.
//! - `skirt_depth` translates the emitted vertices along the side's outward
//!   normal by `+skirt_depth` (i.e. the skirt extends *outward* from the chunk
//!   past the seam). The C++ source emitted vertices at the original side
//!   position and relied on the caller to scale; exposing the depth here makes
//!   the helper self-contained. Pass `0.0` to match the original C++ behavior.
//!
//! ## Indexing convention
//! Same ZXY layout as [`crate::meshers::blocky::mesher::generate_mesh`]:
//! `index = y + sy*(x + sx*z)`, with `pad = 1` outer voxels on every side.
//
// Index-based `for` loops and element-wise copies mirror the C++ source 1:1
// to keep the parity port easy to diff. Clippy's iterator/copy_from_slice
// suggestions would obscure that correspondence, so they're silenced here.
#![allow(clippy::needless_range_loop, clippy::manual_memcpy)]

use crate::constants::cube_tables::{Side, SIDE_NORMALS};
use crate::math::conv::vec3i_to_vec3f;
use crate::math::{Color, Vector2f, Vector3f, Vector3i};
use crate::meshers::blocky::baked_library::{BakedLibrary, AIR_ID};
use crate::meshers::blocky::mesher::BlockyArrays;
use crate::meshers::blocky::{ModelSurface, SideSurface, MAX_SURFACES};

/// Padding the skirt pass assumes on every side of the block. Matches
/// `VoxelMesherBlocky::PADDING` and the `pad` local in the C++ source.
pub const PADDING: i32 = 1;

// ---------------------------------------------------------------------------
// Side-relative helpers (the free functions at the top of the C++ header)
// ---------------------------------------------------------------------------

/// `side_to_block_coordinates` — convert a side-relative coordinate back into
/// block (XYZ) space. Matches the `switch (side)` in the C++ header.
///
/// Note: the C++ source has a commented-out `v.zxy()` and uses `v.yzx()` for
/// the Y sides; this port matches the *active* branch (`yzx`).
#[inline]
fn side_to_block_coordinates(v: Vector3f, side: Side) -> Vector3f {
    match side {
        Side::Left | Side::Right => v.zyx(),
        Side::Bottom | Side::Top => v.yzx(),
        Side::Back | Side::Front => v,
    }
}

/// `get_side_sign` — `+1` for the positive sides, `-1` for the negative ones.
/// Matches the C++ `get_side_sign`.
#[inline]
fn get_side_sign(side: Side) -> i32 {
    match side {
        Side::Right | Side::Bottom | Side::Back => -1,
        Side::Left | Side::Top | Side::Front => 1,
    }
}

// ---------------------------------------------------------------------------
// append_side_skirts / append_skirts
// ---------------------------------------------------------------------------

/// Append skirt geometry for one side of the block. Ported from the C++
/// `append_side_skirts` template.
///
/// * `out` — one [`BlockyArrays`] per material index (same convention as
///   [`crate::meshers::blocky::mesher::generate_mesh`]).
/// * `type_buffer` — flat voxel-id channel, ZXY layout, padded by [`PADDING`].
/// * `jump` — per-axis stride in linear-index units, in side-relative
///   `(x, y, z)` order (i.e. `jump.x` advances one step along the side's X,
///   `jump.y` along the side's Y, `jump.z` along the side's depth axis).
/// * `z` — coordinate (along the side's depth axis) of the first or last voxel
///   plane; this is *not* within the padded region.
/// * `size_x`, `size_y` — extent of the side plane (including padding).
/// * `side` — which block side these skirts belong to.
/// * `skirt_depth` — how far to extend the skirt vertices along the side's
///   outward normal (the skirt is extruded *past* the chunk seam). Pass `0.0`
///   to emit the face geometry in place.
#[allow(clippy::too_many_arguments)]
fn append_side_skirts<T>(
    out: &mut [BlockyArrays],
    type_buffer: &[T],
    jump: Vector3i,
    z: i32,
    size_x: i32,
    size_y: i32,
    side: Side,
    library: &BakedLibrary,
    skirt_depth: f32,
) where
    T: Copy + Into<u16>,
{
    const AIR: u16 = AIR_ID;
    const PAD: i32 = PADDING;

    let z_base = z * jump.z;
    let side_sign = get_side_sign(side);

    // The drop vector applied to every emitted position. Skirts extend the
    // border face *outward* along the side's normal — past the chunk seam — so
    // that a lower-LOD mesh sitting behind it can't show a crack. `skirt_depth`
    // is how far the skirt extends; `0.0` emits the face in place (matching the
    // original C++ behavior, which had no depth parameter).
    let normal = vec3i_to_vec3f(SIDE_NORMALS[side as usize]);
    let drop = normal * skirt_depth;

    // For each outer voxel on the side of the chunk (side-relative coords).
    for x in PAD..(size_x - PAD) {
        for y in PAD..(size_y - PAD) {
            let buffer_index = (x * jump.x + y * jump.y + z_base) as usize;
            let v: u16 = type_buffer[buffer_index].into();

            if v == AIR {
                continue;
            }

            // Check if the voxel is exposed to air along its side-plane
            // neighbors (the four in-plane directions).
            let nv0: u16 = type_buffer[(buffer_index as i32 - jump.x) as usize].into();
            let nv1: u16 = type_buffer[(buffer_index as i32 + jump.x) as usize].into();
            let nv2: u16 = type_buffer[(buffer_index as i32 - jump.y) as usize].into();
            let nv3: u16 = type_buffer[(buffer_index as i32 + jump.y) as usize].into();

            if nv0 != AIR && nv1 != AIR && nv2 != AIR && nv3 != AIR {
                continue;
            }

            // Check if the outer voxel occludes an inner voxel. (This check is
            // not actually accurate, the C++ comment says the same — maybe we'd
            // have to do a full occlusion check using the library.)
            let nv4: u16 = type_buffer[(buffer_index as i32 - side_sign * jump.z) as usize].into();
            if nv4 == AIR {
                continue;
            }

            // If it does, add geometry for the side of that inner voxel.
            // C++:  pos = side_to_block_coordinates(
            //          Vector3f(x - pad, y - pad, z - (side_sign + 1)), side);
            let local = Vector3f::new(
                (x - PAD) as f32,
                (y - PAD) as f32,
                (z - (side_sign + 1)) as f32,
            );
            let pos = side_to_block_coordinates(local, side);

            if (nv4 as usize) >= library.models.len() {
                // Bad ID, skip.
                continue;
            }
            let voxel_baked_data = &library.models[nv4 as usize];

            if !voxel_baked_data.lod_skirts {
                // A typical issue is making an ocean: skirts will show up
                // behind the water surface, so it's not a good solution in
                // that case. Models can opt out via `lod_skirts = false`.
                continue;
            }

            let model = &voxel_baked_data.model;
            let tint: Color = voxel_baked_data.color;

            let side_surfaces: &[SideSurface; MAX_SURFACES] = &model.sides_surfaces[side as usize];
            let model_surfaces: &[ModelSurface; MAX_SURFACES] = &model.surfaces;

            for surface_index in 0..model.surface_count as usize {
                let surface = &model_surfaces[surface_index];
                let material_id = surface.material_id as usize;
                if material_id >= out.len() {
                    continue;
                }
                let arrays = &mut out[material_id];

                let side_surface = &side_surfaces[surface_index];
                let side_positions = &side_surface.positions;
                let vertex_count = side_positions.len();
                let side_uvs = &side_surface.uvs;
                let side_tangents = &side_surface.tangents;
                let side_indices = &side_surface.indices;
                let index_count = side_indices.len();

                // The following code is pretty much the same as the main
                // meshing function (see `generate_mesh`).

                let index_offset = arrays.positions.len();

                // Positions.
                let append_index = arrays.positions.len();
                arrays
                    .positions
                    .resize(append_index + vertex_count, Vector3f::new(0.0, 0.0, 0.0));
                for i in 0..vertex_count {
                    arrays.positions[append_index + i] = side_positions[i] + pos + drop;
                }

                // UVs.
                let uv_append = arrays.uvs.len();
                arrays
                    .uvs
                    .resize(uv_append + vertex_count, Vector2f::new(0.0, 0.0));
                for i in 0..vertex_count {
                    arrays.uvs[uv_append + i] = side_uvs[i];
                }

                // Tangents (4 floats per vertex), if present.
                if !side_tangents.is_empty() {
                    let t_append = arrays.tangents.len();
                    arrays.tangents.resize(t_append + vertex_count * 4, 0.0);
                    for i in 0..vertex_count * 4 {
                        arrays.tangents[t_append + i] = side_tangents[i];
                    }
                }

                // Normals (implicit from the side's normal).
                let n_append = arrays.normals.len();
                arrays
                    .normals
                    .resize(n_append + vertex_count, Vector3f::new(0.0, 0.0, 0.0));
                for i in 0..vertex_count {
                    arrays.normals[n_append + i] = normal;
                }

                // Colors.
                let c_append = arrays.colors.len();
                arrays.colors.resize(c_append + vertex_count, Color::WHITE);
                for i in 0..vertex_count {
                    arrays.colors[c_append + i] = tint;
                }

                // Indices.
                let idx_append = arrays.indices.len();
                arrays.indices.resize(idx_append + index_count, 0);
                for j in 0..index_count {
                    arrays.indices[idx_append + j] = (index_offset as i32) + side_indices[j];
                }
            }
        }
    }
}

/// Append LOD seam-skirt geometry for all six sides of the block. Ported from
/// the C++ `append_skirts` template.
///
/// `out` is one [`BlockyArrays`] per material (sized to at least
/// `library.indexed_materials_count`). `skirt_depth` is how far the skirts
/// extend along each side's outward normal (use `0.0` to match the C++
/// behavior of emitting the side geometry in place).
pub fn append_skirts<T: Copy + Into<u16>>(
    out: &mut [BlockyArrays],
    type_buffer: &[T],
    block_size: Vector3i,
    library: &BakedLibrary,
    skirt_depth: f32,
) {
    // ZXY strides: index = y + sy*(x + sx*z).
    // The C++ source builds `Vector3T<int> jump(size.y, 1, size.x * size.y)`
    // and then passes `.xyz()`, `.zyx()`, or `.zxy()` to each side.
    let jump = Vector3i::new(block_size.y, 1, block_size.x * block_size.y);

    // NEGATIVE_Z plane: depth axis = Z, side plane = (X, Y). Pass jump.xyz.
    append_side_skirts(
        out,
        type_buffer,
        jump.xyz(),
        0,
        block_size.x,
        block_size.y,
        Side::Back,
        library,
        skirt_depth,
    );
    // POSITIVE_Z plane.
    append_side_skirts(
        out,
        type_buffer,
        jump.xyz(),
        block_size.z - 1,
        block_size.x,
        block_size.y,
        Side::Front,
        library,
        skirt_depth,
    );
    // NEGATIVE_X plane: depth axis = X, side plane = (Z, Y). Pass jump.zyx().
    append_side_skirts(
        out,
        type_buffer,
        jump.zyx(),
        0,
        block_size.z,
        block_size.y,
        Side::Right,
        library,
        skirt_depth,
    );
    // POSITIVE_X plane.
    append_side_skirts(
        out,
        type_buffer,
        jump.zyx(),
        block_size.x - 1,
        block_size.z,
        block_size.y,
        Side::Left,
        library,
        skirt_depth,
    );
    // NEGATIVE_Y plane: depth axis = Y, side plane = (Z, X). Pass jump.zxy().
    append_side_skirts(
        out,
        type_buffer,
        jump.zxy(),
        0,
        block_size.z,
        block_size.x,
        Side::Bottom,
        library,
        skirt_depth,
    );
    // POSITIVE_Y plane.
    append_side_skirts(
        out,
        type_buffer,
        jump.zxy(),
        block_size.y - 1,
        block_size.z,
        block_size.x,
        Side::Top,
        library,
        skirt_depth,
    );
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::cube_tables::{CORNER_POSITION, SIDE_CORNERS, SIDE_QUAD_TRIANGLES};
    use crate::meshers::blocky::baked_library::{BakedLibrary, BakedModel};
    use crate::meshers::blocky::SideSurface;

    /// Build a full unit-cube side surface for `side` (4 corners + 2 triangles).
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

    /// A baked library whose model 1 is a full opaque cube that opts into LOD
    /// skirts. Mirrors `mesher::tests::full_cube_library_baked` plus
    /// `lod_skirts = true`.
    fn full_cube_library_with_skirts() -> BakedLibrary {
        let air = BakedModel::default();
        let mut cube = BakedModel {
            empty: false,
            culls_neighbors: true,
            contributes_to_ao: true,
            lod_skirts: true,
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
    fn side_to_block_coordinates_is_identity_for_z_sides() {
        let v = Vector3f::new(1.0, 2.0, 3.0);
        assert_eq!(side_to_block_coordinates(v, Side::Front), v);
        assert_eq!(side_to_block_coordinates(v, Side::Back), v);
        // X sides use the zyx swizzle.
        assert_eq!(
            side_to_block_coordinates(v, Side::Left),
            Vector3f::new(3.0, 2.0, 1.0)
        );
        // Y sides use the yzx swizzle.
        assert_eq!(
            side_to_block_coordinates(v, Side::Top),
            Vector3f::new(2.0, 3.0, 1.0)
        );
    }

    #[test]
    fn get_side_sign_matches_normal_direction() {
        // Positive sides → +1.
        assert_eq!(get_side_sign(Side::Left), 1); // +X
        assert_eq!(get_side_sign(Side::Top), 1); // +Y
        assert_eq!(get_side_sign(Side::Front), 1); // +Z
                                                   // Negative sides → -1.
        assert_eq!(get_side_sign(Side::Right), -1); // -X
        assert_eq!(get_side_sign(Side::Bottom), -1); // -Y
        assert_eq!(get_side_sign(Side::Back), -1); // -Z
    }

    /// A block that is solid cube everywhere produces no skirts: the border
    /// voxels have no air neighbor in-plane, so the "exposed to air" check
    /// rejects them.
    #[test]
    fn append_skirts_solid_block_emits_nothing() {
        let size = Vector3i::new(4, 4, 4);
        let buf = vec![1u16; (size.x * size.y * size.z) as usize];
        let lib = full_cube_library_with_skirts();
        let mut out = vec![BlockyArrays::new()];

        append_skirts(&mut out, &buf, size, &lib, 1.0);

        assert_eq!(out[0].vertex_count(), 0, "solid block needs no skirts");
        assert!(out[0].indices.is_empty());
    }

    /// A single cube at the interior, with air on the border: the border voxels
    /// are all air, so nothing is emitted (skirts only attach to non-air border
    /// voxels).
    #[test]
    fn append_skirts_air_border_emits_nothing() {
        let size = Vector3i::new(3, 3, 3);
        let mut buf = vec![0u16; (size.x * size.y * size.z) as usize];
        let row = size.y;
        let deck = size.x * row;
        let idx = |x: i32, y: i32, z: i32| (y + x * row + z * deck) as usize;
        buf[idx(1, 1, 1)] = 1; // single interior cube
        let lib = full_cube_library_with_skirts();
        let mut out = vec![BlockyArrays::new()];

        append_skirts(&mut out, &buf, size, &lib, 1.0);

        // Border is entirely air → no border voxel is non-air → no skirts.
        assert_eq!(out[0].vertex_count(), 0);
    }

    /// Build a buffer with a single non-air voxel on the −Z border (z=0) that
    /// is *exposed* (all four in-plane neighbors are air) and has a non-air
    /// inner neighbor at z=1. This is the minimal setup that triggers the
    /// skirt-emission path for the −Z (Back) side.
    ///
    /// We use a 4×4×4 block so the iterated border region `x in 1..3` has room
    /// for a voxel whose in-plane (±x, ±y) neighbors are all air.
    fn exposed_border_buffer() -> (Vector3i, Vec<u16>) {
        let size = Vector3i::new(4, 4, 4);
        let mut buf = vec![0u16; (size.x * size.y * size.z) as usize];
        let row = size.y;
        let deck = size.x * row;
        let idx = |x: i32, y: i32, z: i32| (y + x * row + z * deck) as usize;
        // Border voxel on the −Z plane, exposed to air in-plane.
        buf[idx(1, 1, 0)] = 1;
        // Inner neighbor toward +Z (the voxel whose +Z face we skirt).
        buf[idx(1, 1, 1)] = 1;
        (size, buf)
    }

    /// A non-air voxel on the −Z border that is exposed to air in-plane and has
    /// a non-air inner neighbor must produce skirt geometry for the inner
    /// voxel's +Z (Front) face.
    #[test]
    fn append_skirts_emits_for_exposed_border_voxel() {
        let (size, buf) = exposed_border_buffer();
        let lib = full_cube_library_with_skirts();
        let mut out = vec![BlockyArrays::new()];
        append_skirts(&mut out, &buf, size, &lib, 0.0);

        // The border voxel (1,1,0) is non-air, has all-air in-plane neighbors,
        // and its inner neighbor (1,1,1) is non-air with lod_skirts enabled →
        // a skirt quad (4 verts, 6 indices) must be emitted.
        assert!(
            out[0].vertex_count() > 0,
            "expected skirt geometry for an exposed border voxel"
        );
        assert!(
            out[0].indices.len() % 3 == 0,
            "index count must be a multiple of 3"
        );
        // Every emitted normal is the side's outward normal (a unit axis vec).
        for n in &out[0].normals {
            let is_axis_unit = (n.x.abs() == 1.0 && n.y == 0.0 && n.z == 0.0)
                || (n.y.abs() == 1.0 && n.x == 0.0 && n.z == 0.0)
                || (n.z.abs() == 1.0 && n.x == 0.0 && n.y == 0.0);
            assert!(is_axis_unit, "skirt normals must be unit axis vectors");
        }
    }

    /// `lod_skirts = false` on the model suppresses all skirt geometry even
    /// when the border voxel is exposed.
    #[test]
    fn append_skirts_respects_lod_skirts_flag() {
        let (size, buf) = exposed_border_buffer();
        let mut lib = full_cube_library_with_skirts();
        lib.models[1].lod_skirts = false;

        let mut out = vec![BlockyArrays::new()];
        append_skirts(&mut out, &buf, size, &lib, 0.0);

        assert_eq!(
            out[0].vertex_count(),
            0,
            "lod_skirts = false must suppress all skirt geometry"
        );
    }

    /// `skirt_depth > 0` must offset emitted vertices along the side's outward
    /// normal. For the −Z border skirt, the inner neighbor's Back (−Z) side
    /// geometry is emitted; with depth `d` every position shifts by
    /// `d * normal(−Z)` = `(0, 0, -d)` (the skirt extends outward from the
    /// chunk along −Z).
    #[test]
    fn append_skirts_drops_vertices_by_depth() {
        let (size, buf) = exposed_border_buffer();
        let lib = full_cube_library_with_skirts();

        // depth = 0 baseline.
        let mut out0 = vec![BlockyArrays::new()];
        append_skirts(&mut out0, &buf, size, &lib, 0.0);

        // depth = 2.0.
        let mut out2 = vec![BlockyArrays::new()];
        append_skirts(&mut out2, &buf, size, &lib, 2.0);

        // Same vertex count (depth doesn't add geometry, just offsets it).
        assert_eq!(out0[0].vertex_count(), out2[0].vertex_count());
        assert!(out0[0].vertex_count() > 0);

        // The only side that emits here is Back (−Z border), whose normal is
        // (0,0,−1); the drop is `depth * (0,0,−1)` = (0,0,−2). So every
        // depth-2 vertex sits 2 units below its depth-0 counterpart in Z.
        for (a, b) in out0[0].positions.iter().zip(out2[0].positions.iter()) {
            assert!(
                (*a - *b).z > 1.99,
                "depth must extend the vertex along -Z, got a={a:?} b={b:?}"
            );
        }
    }
}
