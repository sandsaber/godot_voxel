//! Transvoxel transition-cell mesh extraction.
//!
//! Faithful port of `build_transition_mesh` from `meshers/transvoxel/transvoxel.cpp`
//! (lines ~706-1090). Produces the LOD-seam mesh that stitches a high-resolution
//! block to its low-resolution neighbour so the rendered surface stays
//! watertight across LOD boundaries.
//!
//! Phase 0 implements `TEXTURES_NONE` mode only (no mixel4 / single_s4 material
//! blending). Transition verts are appended to the same [`MeshArrays`] the
//! regular-cell mesher wrote into.

// `RegularMesherInput::len` mirrors `Span::len` and intentionally has no
// `is_empty`; the indexing loop in `cell_samples` is clearer than an iterator.
// `identity_op` fires on the explicit `fx + 0`/`fy + 0` calls below, kept on
// purpose to mirror the C++ `face_to_block(fx + i, fy + j, fz, ...)` 3x3 grid.
#![allow(
    clippy::len_without_is_empty,
    clippy::needless_range_loop,
    clippy::identity_op
)]

use super::regular::{BuildRegularMeshParams, RegularMesherInput, MAX_PADDING, MIN_PADDING};
use super::structures::{Cache, MeshArrays};
use super::transition_tables;
use crate::math::funcs;
use crate::math::{Vector3f, Vector3i};

// Axis indices, matching C++ `Vector3i::AXIS_X/Y/Z`.
const AXIS_X: usize = 0;
const AXIS_Y: usize = 1;
const AXIS_Z: usize = 2;

/// Cube::Side direction constants, matching `Cube::Side` (cube_tables.h).
/// `SIDE_NEGATIVE_X` is an alias for `SIDE_LEFT` (=0). The numbers must agree
/// with the C++ `face_to_block` / `get_face_axes` / `get_face_index` switches.
pub const SIDE_NEGATIVE_X: u8 = 0;
pub const SIDE_POSITIVE_X: u8 = 1;
pub const SIDE_NEGATIVE_Y: u8 = 2;
pub const SIDE_POSITIVE_Y: u8 = 3;
pub const SIDE_NEGATIVE_Z: u8 = 4;
pub const SIDE_POSITIVE_Z: u8 = 5;
/// Number of cube sides (matches C++ `Cube::SIDE_COUNT`).
pub const SIDE_COUNT: u8 = 6;

/// Scale factor applied to border offsets when computing secondary positions.
/// Matches `TRANSITION_CELL_SCALE` in transvoxel.cpp:22.
const TRANSITION_CELL_SCALE: f32 = 0.25;

// ---------------------------------------------------------------------------
// Helpers (ported from transvoxel.cpp lines 611-702)
// ---------------------------------------------------------------------------

/// SDF sign bit: 1 if the sample is negative (inside solid).
/// Matches C++ `sign_f(float v) { return v < 0.f; }`.
#[inline]
fn sign_f(v: f32) -> u8 {
    (v < 0.0) as u8
}

/// Convert from face-space to block-space coordinates, considering which face
/// we are working on. Matches C++ `face_to_block(x, y, z, dir, bs)`.
#[inline]
fn face_to_block(x: i32, y: i32, z: i32, dir: u8, bs: Vector3i) -> Vector3i {
    match dir {
        SIDE_NEGATIVE_X => Vector3i::new(z, x, y),
        SIDE_POSITIVE_X => Vector3i::new(bs.x - 1 - z, y, x),
        SIDE_NEGATIVE_Y => Vector3i::new(y, z, x),
        SIDE_POSITIVE_Y => Vector3i::new(x, bs.y - 1 - z, y),
        SIDE_NEGATIVE_Z => Vector3i::new(x, y, z),
        SIDE_POSITIVE_Z => Vector3i::new(y, x, bs.z - 1 - z),
        _ => Vector3i::zero(),
    }
}

/// Returns the two face-axes (`ax`, `ay`) used for face-space iteration.
/// Matches C++ `get_face_axes`.
#[inline]
fn get_face_axes(dir: u8) -> (usize, usize) {
    match dir {
        SIDE_NEGATIVE_X => (AXIS_Y, AXIS_Z),
        SIDE_POSITIVE_X => (AXIS_Z, AXIS_Y),
        SIDE_NEGATIVE_Y => (AXIS_Z, AXIS_X),
        SIDE_POSITIVE_Y => (AXIS_X, AXIS_Z),
        SIDE_NEGATIVE_Z => (AXIS_X, AXIS_Y),
        SIDE_POSITIVE_Z => (AXIS_Y, AXIS_X),
        _ => (AXIS_X, AXIS_Y),
    }
}

/// Returns the per-face index 0..6 used as a hint bit in `transition_hint_mask`.
/// Matches C++ `get_face_index`.
#[inline]
fn get_face_index(cube_dir: u8) -> u8 {
    match cube_dir {
        SIDE_NEGATIVE_X => 0,
        SIDE_POSITIVE_X => 1,
        SIDE_NEGATIVE_Y => 2,
        SIDE_POSITIVE_Y => 3,
        SIDE_NEGATIVE_Z => 4,
        SIDE_POSITIVE_Z => 5,
        _ => 0,
    }
}

/// Normalize, returning (0,1,0) for a zero-length input. Matches `normalized_not_null`.
#[inline]
fn normalized_not_null(n: Vector3f) -> Vector3f {
    let lsq = n.x * n.x + n.y * n.y + n.z * n.z;
    if lsq == 0.0 {
        Vector3f::new(0.0, 1.0, 0.0)
    } else {
        let l = funcs::sqrt_f32(lsq);
        Vector3f::new(n.x / l, n.y / l, n.z / l)
    }
}

/// Compute a 6-bit border mask for a position within a block.
/// Bits: 1=-X 2=+X 4=-Y 8=+Y 16=-Z 32=+Z. Matches `get_border_mask`.
#[inline]
fn get_border_mask(pos: Vector3i, block_size: Vector3i) -> u8 {
    let mut mask = 0u8;
    for (axis, (p, s)) in [pos.x, pos.y, pos.z]
        .iter()
        .zip([block_size.x, block_size.y, block_size.z].iter())
        .enumerate()
    {
        if *p == 0 {
            mask |= 1 << (axis * 2);
        }
        if *p == *s {
            mask |= 1 << (axis * 2 + 1);
        }
    }
    mask
}

/// Convert a `Vector3i` to `Vector3f`. Matches `to_vec3f(Vector3i)`.
#[inline]
fn to_vec3f(v: Vector3i) -> Vector3f {
    Vector3f::new(v.x as f32, v.y as f32, v.z as f32)
}

/// Multiply each component by `1 << lod`. Matches C++ `Vector3i << lod_index`.
#[inline]
fn scale_for_lod(v: Vector3i, lod_index: u32) -> Vector3i {
    let s = (1i32) << lod_index;
    Vector3i::new(v.x * s, v.y * s, v.z * s)
}

// ---------------------------------------------------------------------------
// Secondary-position helpers (duplicated from regular.rs since they are
// private there; matches transvoxel.cpp:29-92 verbatim).
// ---------------------------------------------------------------------------

/// Secondary position for LOD transitions. Matches `get_secondary_position`.
fn get_secondary_position(
    primary: Vector3f,
    normal: Vector3f,
    lod_index: u32,
    block_size_non_scaled: Vector3i,
) -> Vector3f {
    let mut delta = get_border_offset(primary, lod_index, block_size_non_scaled);
    delta = project_border_offset(delta, normal);

    // Clamp to ±2^lod to avoid shooting far at very high LOD.
    let p2k = (1u32 << lod_index) as f32;
    delta = Vector3f::new(
        funcs::clampf(delta.x, -p2k, p2k),
        funcs::clampf(delta.y, -p2k, p2k),
        funcs::clampf(delta.z, -p2k, p2k),
    );

    primary + delta
}

/// Matches `get_border_offset`.
fn get_border_offset(
    pos_scaled: Vector3f,
    lod_index: u32,
    block_size_non_scaled: Vector3i,
) -> Vector3f {
    let mut delta = [0.0f32; 3];
    let p2k = (1u32 << lod_index) as f32;
    let p2mk = 1.0 / p2k;
    let wk = TRANSITION_CELL_SCALE * p2k;

    let p_arr = [pos_scaled.x, pos_scaled.y, pos_scaled.z];
    let s_arr = [
        block_size_non_scaled.x as f32,
        block_size_non_scaled.y as f32,
        block_size_non_scaled.z as f32,
    ];

    for i in 0..3 {
        let p = p_arr[i];
        let s = s_arr[i];
        if p < p2k {
            delta[i] = (1.0 - p2mk * p) * wk;
        } else if p > p2k * (s - 1.0) {
            delta[i] = (s - 1.0 - p2mk * p) * wk;
        }
    }
    Vector3f::new(delta[0], delta[1], delta[2])
}

/// Matches `project_border_offset`.
fn project_border_offset(delta: Vector3f, normal: Vector3f) -> Vector3f {
    Vector3f::new(
        (1.0 - normal.x * normal.x) * delta.x
            - normal.y * normal.x * delta.y
            - normal.z * normal.x * delta.z,
        -normal.x * normal.y * delta.x + (1.0 - normal.y * normal.y) * delta.y
            - normal.z * normal.y * delta.z,
        -normal.x * normal.z * delta.x - normal.y * normal.z * delta.y
            + (1.0 - normal.z * normal.z) * delta.z,
    )
}

// ---------------------------------------------------------------------------
// Core algorithm
// ---------------------------------------------------------------------------

/// Extract a transition-cell surface mesh from one face of an SDF voxel block.
///
/// Port of `build_transition_mesh` (transvoxel.cpp:706). Vertices and indices
/// are *appended* to `output` — call this after [`super::regular::build_regular_mesh`]
/// so the regular and transition geometry share the same [`MeshArrays`].
///
/// `direction` is a `Cube::SIDE_*` constant (see [`SIDE_NEGATIVE_X`] et al.).
/// The transition mesh uses the block's own SDF data; it does not need the
/// neighbour's voxels.
///
/// `TEXTURES_NONE` mode only: no material processor, no texture data (matching
/// the regular-cell Phase 0 port).
pub fn build_transition_mesh(
    input: &dyn RegularMesherInput,
    params: &BuildRegularMeshParams,
    direction: u8,
    cache: &mut Cache,
    output: &mut MeshArrays,
) {
    let lod_index = params.lod_index;
    let edge_clamp_margin = params.edge_clamp_margin;
    let edge_clamp_margin_max = 1.0 - edge_clamp_margin;

    let block_size_with_padding = input.block_size();

    if block_size_with_padding.x < 3
        || block_size_with_padding.y < 3
        || block_size_with_padding.z < 3
    {
        return;
    }

    // The actual block (without padding). Matches:
    //   block_size_without_padding = block_size_with_padding - (MIN_PADDING + MAX_PADDING)
    let block_size_without_padding = Vector3i::new(
        block_size_with_padding.x - (MIN_PADDING + MAX_PADDING),
        block_size_with_padding.y - (MIN_PADDING + MAX_PADDING),
        block_size_with_padding.z - (MIN_PADDING + MAX_PADDING),
    );
    let block_size_scaled = scale_for_lod(block_size_without_padding, lod_index);

    cache.reset_reuse_cells_2d(block_size_with_padding);

    // This represents the actual box of voxels we are working on.
    // Padding is present to allow reaching 1 voxel further for calculating normals.
    let min_pos = Vector3i::splat(MIN_PADDING);
    let max_pos = Vector3i::new(
        block_size_with_padding.x - MAX_PADDING,
        block_size_with_padding.y - MAX_PADDING,
        block_size_with_padding.z - MAX_PADDING,
    );

    let (axis_x, axis_y) = get_face_axes(direction);
    let min_fpos_x = min_pos[axis_x];
    let min_fpos_y = min_pos[axis_y];
    // Another -1 here, because the 2D kernel is 3x3 (reaches two voxels ahead).
    let max_fpos_x = max_pos[axis_x] - 1;
    let max_fpos_y = max_pos[axis_y] - 1;

    // How much to advance in the data array to get neighbor voxels in block space.
    // ZXY memory layout: index = y + sy*(x + sx*z).
    let sy = block_size_with_padding.y as usize;
    let sx = block_size_with_padding.x as usize;
    let n010 = 1usize; // Y+1 (Y innermost)
    let n100 = sy; // X+1
    let n001 = sy * sx; // Z+1

    // Using temporary locals otherwise the arithmetic gets hard to read.
    // These convert a step in face-space into a step in the flat data array.
    let ftb_000 = face_to_block(0, 0, 0, direction, block_size_with_padding);
    let ftb_x00 = face_to_block(1, 0, 0, direction, block_size_with_padding);
    let ftb_0y0 = face_to_block(0, 1, 0, direction, block_size_with_padding);

    // `fn00` is the absolute base index of the face-space origin (0,0); we
    // only need the *deltas* `fn10`/`fn01` to step in face space, so the
    // base is subtracted out and discarded.
    let fn00 = ftb_000.zxy_index(block_size_with_padding) as i32;
    let fn10 = ftb_x00.zxy_index(block_size_with_padding) as i32 - fn00;
    let fn01 = ftb_0y0.zxy_index(block_size_with_padding) as i32 - fn00;
    let fn11 = fn10 + fn01;
    let fn21 = 2 * fn10 + fn01;
    let fn22 = 2 * fn10 + 2 * fn01;
    let fn12 = fn10 + 2 * fn01;
    let fn20 = 2 * fn10;
    let fn02 = 2 * fn01;

    // Each face iteration uses a fixed z-slice (the minimum-padding slice).
    let fz = MIN_PADDING;

    let isolevel: f32 = 0.0;

    // Bit set in `LodAttrib.transition` for vertices on this face. Downstream
    // skinning code uses it to know which vertices belong to a LOD seam.
    let transition_hint_mask: u8 = 1 << get_face_index(direction);

    // Iterate in face space. Step by 2 because each transition cell spans a
    // 3x3 patch of full-resolution samples (the central sample is shared
    // between the four surrounding transition cells).
    let mut fy = min_fpos_y;
    while fy < max_fpos_y {
        let mut fx = min_fpos_x;
        while fx < max_fpos_x {
            // Cell origin in block space (still padded at this point).
            let cp0 = face_to_block(fx, fy, fz, direction, block_size_with_padding);
            let data_index = cp0.zxy_index(block_size_with_padding) as usize;

            // ---- Early-out: skip cells that don't cross the isolevel ----
            // Note: `sample_f32()` already returns the signed-distance
            // convention used by the algorithm (negated from raw storage),
            // so the sign-flip implicit in C++ `sdf_as_float` is already done.
            // C++ checks `sdf_data[i] > isolevel`; after the sign flip the
            // faithful comparison is `< isolevel` (matches regular.rs).
            let s = input.sample_f32(data_index) < isolevel;
            let all_same = (input.sample_f32((data_index as i32 + fn10) as usize) < isolevel) == s
                && (input.sample_f32((data_index as i32 + fn20) as usize) < isolevel) == s
                && (input.sample_f32((data_index as i32 + fn01) as usize) < isolevel) == s
                && (input.sample_f32((data_index as i32 + fn11) as usize) < isolevel) == s
                && (input.sample_f32((data_index as i32 + fn21) as usize) < isolevel) == s
                && (input.sample_f32((data_index as i32 + fn02) as usize) < isolevel) == s
                && (input.sample_f32((data_index as i32 + fn12) as usize) < isolevel) == s
                && (input.sample_f32((data_index as i32 + fn22) as usize) < isolevel) == s;
            if all_same {
                fx += 2;
                continue;
            }

            // Data indices for the 3x3 patch of full-resolution samples.
            // Layout:
            //   6---7---8
            //   |   |   |
            //   3---4---5
            //   |   |   |
            //   0---1---2
            let cell_data_indices: [usize; 9] = [
                data_index,
                (data_index as i32 + fn10) as usize,
                (data_index as i32 + fn20) as usize,
                (data_index as i32 + fn01) as usize,
                (data_index as i32 + fn11) as usize,
                (data_index as i32 + fn21) as usize,
                (data_index as i32 + fn02) as usize,
                (data_index as i32 + fn12) as usize,
                (data_index as i32 + fn22) as usize,
            ];

            // Full-resolution samples 0..8.
            let mut cell_samples = [0.0f32; 13];
            for i in 0..9 {
                cell_samples[i] = input.sample_f32(cell_data_indices[i]);
            }

            // Half-resolution samples 9..C: they are the same as the corners.
            //   B-------C
            //   |       |
            //   9-------A
            cell_samples[0x9] = cell_samples[0];
            cell_samples[0xA] = cell_samples[2];
            cell_samples[0xB] = cell_samples[6];
            cell_samples[0xC] = cell_samples[8];

            // Build the 9-bit case code. Note: the bit ordering is NOT the same
            // as the sampling order (see the Transvoxel paper for details).
            let mut case_code: u16 = sign_f(cell_samples[0]) as u16;
            case_code |= (sign_f(cell_samples[1]) as u16) << 1;
            case_code |= (sign_f(cell_samples[2]) as u16) << 2;
            case_code |= (sign_f(cell_samples[5]) as u16) << 3;
            case_code |= (sign_f(cell_samples[8]) as u16) << 4;
            case_code |= (sign_f(cell_samples[7]) as u16) << 5;
            case_code |= (sign_f(cell_samples[6]) as u16) << 6;
            case_code |= (sign_f(cell_samples[3]) as u16) << 7;
            case_code |= (sign_f(cell_samples[4]) as u16) << 8;

            if case_code == 0 || case_code == 511 {
                // The cell contains no triangles.
                fx += 2;
                continue;
            }

            debug_assert!(case_code <= 511);

            // TEXTURES_NONE: `material_processor.on_transition_cell` is a no-op,
            // so `ReuseTransitionCell::packed_texture_indices` stays at its
            // default value (0) for every cell. The C++ reuse check
            // `prev.packed_texture_indices == current.packed_texture_indices`
            // therefore always succeeds and we can elide the field entirely.

            // Gradients at each of the 9 full-resolution samples (central
            // differences). Reused for both endpoints of every edge.
            let mut cell_gradients = [Vector3f::zero(); 13];
            for i in 0..9 {
                let di = cell_data_indices[i];
                let nx = input.sample_f32(di - n100);
                let ny = input.sample_f32(di - n010);
                let nz = input.sample_f32(di - n001);
                let px = input.sample_f32(di + n100);
                let py = input.sample_f32(di + n010);
                let pz = input.sample_f32(di + n001);
                cell_gradients[i] = Vector3f::new(nx - px, ny - py, nz - pz);
            }
            cell_gradients[0x9] = cell_gradients[0];
            cell_gradients[0xA] = cell_gradients[2];
            cell_gradients[0xB] = cell_gradients[6];
            cell_gradients[0xC] = cell_gradients[8];

            // Cell-corner positions in block space, un-padded, scaled by LOD.
            let mut cell_positions = [Vector3i::zero(); 13];
            cell_positions[0] =
                face_to_block(fx + 0, fy + 0, fz, direction, block_size_with_padding);
            cell_positions[1] =
                face_to_block(fx + 1, fy + 0, fz, direction, block_size_with_padding);
            cell_positions[2] =
                face_to_block(fx + 2, fy + 0, fz, direction, block_size_with_padding);
            cell_positions[3] =
                face_to_block(fx + 0, fy + 1, fz, direction, block_size_with_padding);
            cell_positions[4] =
                face_to_block(fx + 1, fy + 1, fz, direction, block_size_with_padding);
            cell_positions[5] =
                face_to_block(fx + 2, fy + 1, fz, direction, block_size_with_padding);
            cell_positions[6] =
                face_to_block(fx + 0, fy + 2, fz, direction, block_size_with_padding);
            cell_positions[7] =
                face_to_block(fx + 1, fy + 2, fz, direction, block_size_with_padding);
            cell_positions[8] =
                face_to_block(fx + 2, fy + 2, fz, direction, block_size_with_padding);
            for i in 0..9 {
                cell_positions[i] = scale_for_lod(cell_positions[i] - min_pos, lod_index);
            }
            cell_positions[0x9] = cell_positions[0];
            cell_positions[0xA] = cell_positions[2];
            cell_positions[0xB] = cell_positions[6];
            cell_positions[0xC] = cell_positions[8];

            let cell_class = transition_tables::get_transition_cell_class(case_code as usize);
            let class_index = (cell_class & 0x7f) as usize;
            debug_assert!(class_index <= 55);

            let cell_data = *transition_tables::get_transition_cell_data(class_index);
            let flip_triangles = (cell_class & 128) != 0;

            let vertex_count = cell_data.vertex_count() as usize;
            let triangle_count = cell_data.triangle_count() as usize;
            let mut cell_vertex_indices = [-1i32; 12];
            debug_assert!(vertex_count <= cell_vertex_indices.len());

            // Mask of which preceding neighbours exist. Bit 0 = -X available,
            // bit 1 = -Y available. Cells on the low end of the face don't have
            // a preceding neighbour on that axis.
            let direction_validity_mask: u8 =
                (if fx > min_fpos_x { 1 } else { 0 }) | (if fy > min_fpos_y { 1 } else { 0 } << 1);

            let cell_border_mask: u8 = get_border_mask(cell_positions[0], block_size_scaled);

            // ---- For each vertex produced by this cell ----
            for vertex_index in 0..vertex_count {
                let edge_code =
                    transition_tables::get_transition_vertex_data(case_code as usize, vertex_index);
                let index_vertex_a = ((edge_code >> 4) & 0xf) as usize;
                let index_vertex_b = (edge_code & 0xf) as usize;

                let sample_a = cell_samples[index_vertex_a];
                let sample_b = cell_samples[index_vertex_b];

                // Degenerate edges should not appear in the tables, but guard
                // against divide-by-zero just in case.
                if sample_a == sample_b || (sample_a == 0.0 && sample_b == 0.0) {
                    continue;
                }

                // Interpolation parameter along the edge.
                let t = funcs::clampf(
                    sample_b / (sample_b - sample_a),
                    edge_clamp_margin,
                    edge_clamp_margin_max,
                );

                if t > 0.0 && t < 1.0 {
                    // Vertex lies in the interior of the edge.
                    let vertex_index_to_reuse_or_create = ((edge_code >> 8) & 0xf) as usize;
                    // Bit 0 (0x1): need to subtract one to X
                    // Bit 1 (0x2): need to subtract one to Y
                    // Bit 2 (0x4): vertex is on an interior edge, won't be reused
                    // Bit 3 (0x8): vertex is on a maximal edge, it can be reused
                    let reuse_direction = ((edge_code >> 12) & 0xf) as u8;

                    let present = (reuse_direction & direction_validity_mask) == reuse_direction;

                    if present {
                        let prev = cache.get_reuse_cell_2d(
                            (fx - (reuse_direction & 1) as i32) as usize,
                            (fy - ((reuse_direction >> 1) & 1) as i32) as usize,
                        );
                        // TEXTURES_NONE: the texture-index equality check is
                        // always true (both sides default to 0), so reuse is
                        // always permitted.
                        cell_vertex_indices[vertex_index] =
                            prev.vertices[vertex_index_to_reuse_or_create];
                    }

                    if !present || cell_vertex_indices[vertex_index] == -1 {
                        let p0 = cell_positions[index_vertex_a];
                        let p1 = cell_positions[index_vertex_b];
                        let n0 = cell_gradients[index_vertex_a];
                        let n1 = cell_gradients[index_vertex_b];

                        let t0 = t;
                        let t1 = 1.0 - t;
                        let primaryf = to_vec3f(p0) * t0 + to_vec3f(p1) * t1;
                        let normal = normalized_not_null(n0 * t0 + n1 * t1);

                        // A vertex is on the "full-resolution" side if either
                        // endpoint is one of the 9 full-res samples (<9). The
                        // half-res side (9..C) is the side of the block where
                        // we don't want vertices to move.
                        let fullres_side = index_vertex_a < 9 || index_vertex_b < 9;

                        let (secondary, vertex_border_mask, cell_border_mask2): (Vector3f, u8, u8) =
                            if fullres_side {
                                let sec = get_secondary_position(
                                    primaryf,
                                    normal,
                                    lod_index,
                                    block_size_without_padding,
                                );
                                let vbm = get_border_mask(p0, block_size_scaled)
                                    & get_border_mask(p1, block_size_scaled);
                                (sec, vbm, cell_border_mask)
                            } else {
                                // Half-res side: don't move the vertex, and clear
                                // cell_border_mask so the regular-mesh skinning
                                // does not pull these vertices either.
                                (Vector3f::zero(), 0u8, 0u8)
                            };

                        let vi = output.add_vertex(
                            primaryf,
                            normal,
                            cell_border_mask2,
                            vertex_border_mask,
                            transition_hint_mask,
                            secondary,
                        );
                        cell_vertex_indices[vertex_index] = vi;

                        if (reuse_direction & 0x8) != 0 {
                            // The vertex can be re-used by later cells.
                            let r = cache.get_reuse_cell_2d_mut(fx as usize, fy as usize);
                            r.vertices[vertex_index_to_reuse_or_create] = vi;
                        }
                    }
                } else {
                    // The vertex is exactly on one of the edge endpoints.
                    // Use the reuse information in `transitionCornerData`.
                    let cell_index = if t == 0.0 {
                        index_vertex_b
                    } else {
                        index_vertex_a
                    };
                    debug_assert!(cell_index < 13);

                    let corner_data = transition_tables::get_transition_corner_data(cell_index);
                    let vertex_index_to_reuse_or_create = (corner_data & 0x0f) as usize;
                    let reuse_direction = (corner_data >> 4) & 0x0f;

                    let present = (reuse_direction & direction_validity_mask) == reuse_direction;

                    if present {
                        let prev = cache.get_reuse_cell_2d(
                            (fx - (reuse_direction & 1) as i32) as usize,
                            (fy - ((reuse_direction >> 1) & 1) as i32) as usize,
                        );
                        cell_vertex_indices[vertex_index] =
                            prev.vertices[vertex_index_to_reuse_or_create];
                    }

                    if !present || cell_vertex_indices[vertex_index] == -1 {
                        let primary = cell_positions[cell_index];
                        let primaryf = to_vec3f(primary);
                        let normal = normalized_not_null(cell_gradients[cell_index]);

                        let fullres_side = cell_index < 9;

                        let (secondary, vertex_border_mask, cell_border_mask2): (Vector3f, u8, u8) =
                            if fullres_side {
                                let sec = get_secondary_position(
                                    primaryf,
                                    normal,
                                    lod_index,
                                    block_size_without_padding,
                                );
                                (
                                    sec,
                                    get_border_mask(primary, block_size_scaled),
                                    cell_border_mask,
                                )
                            } else {
                                (Vector3f::zero(), 0u8, 0u8)
                            };

                        let vi = output.add_vertex(
                            primaryf,
                            normal,
                            cell_border_mask2,
                            vertex_border_mask,
                            transition_hint_mask,
                            secondary,
                        );
                        cell_vertex_indices[vertex_index] = vi;

                        // We are on a corner so the vertex is always re-usable.
                        let r = cache.get_reuse_cell_2d_mut(fx as usize, fy as usize);
                        r.vertices[vertex_index_to_reuse_or_create] = vi;
                    }
                }
            } // for each cell vertex

            // ---- Emit triangles ----
            for ti in 0..triangle_count {
                let base = ti * 3;
                if flip_triangles {
                    output
                        .indices
                        .push(cell_vertex_indices[cell_data.get_vertex_index(base) as usize]);
                    output
                        .indices
                        .push(cell_vertex_indices[cell_data.get_vertex_index(base + 1) as usize]);
                    output
                        .indices
                        .push(cell_vertex_indices[cell_data.get_vertex_index(base + 2) as usize]);
                } else {
                    output
                        .indices
                        .push(cell_vertex_indices[cell_data.get_vertex_index(base + 2) as usize]);
                    output
                        .indices
                        .push(cell_vertex_indices[cell_data.get_vertex_index(base + 1) as usize]);
                    output
                        .indices
                        .push(cell_vertex_indices[cell_data.get_vertex_index(base) as usize]);
                }
            }

            fx += 2;
        } // for fx
        fy += 2;
    } // for fy
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vector3i;

    /// Simple flat SDF buffer used by the transition-mesh tests. Stores the
    /// already-converted signed-distance values (positive outside, negative
    /// inside) in a ZXY-layout `Vec<f32>`.
    struct FlatSdfInput {
        size: Vector3i,
        data: Vec<f32>,
    }

    impl FlatSdfInput {
        fn new_uniform(size: Vector3i, value: f32) -> Self {
            let len = (size.volume_u64()) as usize;
            Self {
                size,
                data: vec![value; len],
            }
        }

        /// Build a buffer whose voxels hold the signed distance to a sphere of
        /// radius `r` centred at `center` (in voxel coordinates, padded space).
        /// Positive outside, negative inside — the convention `sample_f32`
        /// already returns.
        fn new_sphere(size: Vector3i, center: Vector3i, radius: f32) -> Self {
            let mut data = vec![0.0f32; size.volume_u64() as usize];
            let sy = size.y as usize;
            let sx = size.x as usize;
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let dx = x as f32 - center.x as f32;
                        let dy = y as f32 - center.y as f32;
                        let dz = z as f32 - center.z as f32;
                        let d = funcs::sqrt_f32(dx * dx + dy * dy + dz * dz);
                        let idx = (y as usize) + sy * ((x as usize) + sx * (z as usize));
                        data[idx] = d - radius;
                    }
                }
            }
            Self { size, data }
        }
    }

    impl RegularMesherInput for FlatSdfInput {
        fn len(&self) -> usize {
            self.data.len()
        }

        fn block_size(&self) -> Vector3i {
            self.size
        }

        fn sample_f32(&self, data_index: usize) -> f32 {
            self.data[data_index]
        }
    }

    const ALL_DIRECTIONS: [u8; 6] = [
        SIDE_NEGATIVE_X,
        SIDE_POSITIVE_X,
        SIDE_NEGATIVE_Y,
        SIDE_POSITIVE_Y,
        SIDE_NEGATIVE_Z,
        SIDE_POSITIVE_Z,
    ];

    #[test]
    fn uniform_air_produces_zero_triangles() {
        // All-air block: SDF uniformly positive, no isosurface crossing.
        let size = Vector3i::new(8, 8, 8);
        let input = FlatSdfInput::new_uniform(size, 1.0);
        let params = BuildRegularMeshParams::default();
        for dir in ALL_DIRECTIONS {
            let mut cache = Cache::default();
            let mut output = MeshArrays::default();
            build_transition_mesh(&input, &params, dir, &mut cache, &mut output);
            assert_eq!(
                output.indices.len(),
                0,
                "direction {dir}: expected 0 triangles for uniform air"
            );
            assert_eq!(
                output.vertices.len(),
                0,
                "direction {dir}: expected 0 vertices for uniform air"
            );
        }
    }

    #[test]
    fn uniform_solid_produces_zero_triangles() {
        // All-solid block: SDF uniformly negative, no isosurface crossing.
        let size = Vector3i::new(8, 8, 8);
        let input = FlatSdfInput::new_uniform(size, -1.0);
        let params = BuildRegularMeshParams::default();
        for dir in ALL_DIRECTIONS {
            let mut cache = Cache::default();
            let mut output = MeshArrays::default();
            build_transition_mesh(&input, &params, dir, &mut cache, &mut output);
            assert_eq!(
                output.indices.len(),
                0,
                "direction {dir}: expected 0 triangles for uniform solid"
            );
        }
    }

    #[test]
    fn sphere_produces_triangles_on_some_faces() {
        // A sphere centred in the block crosses several faces; at least one
        // direction must produce non-zero geometry. We don't assert an exact
        // count (the value depends on sphere placement relative to padding),
        // only that the algorithm emits *something*.
        let size = Vector3i::new(8, 8, 8);
        let center = Vector3i::new(4, 4, 4);
        let input = FlatSdfInput::new_sphere(size, center, 3.0);
        let params = BuildRegularMeshParams::default();

        let mut total_triangles = 0usize;
        for dir in ALL_DIRECTIONS {
            let mut cache = Cache::default();
            let mut output = MeshArrays::default();
            build_transition_mesh(&input, &params, dir, &mut cache, &mut output);
            // Each triangle is 3 indices.
            total_triangles += output.indices.len() / 3;
        }
        assert!(
            total_triangles > 0,
            "expected the sphere to produce some transition triangles, got 0"
        );
    }

    #[test]
    fn all_directions_run_without_panic() {
        // Smoke test: every direction must complete without panicking on a
        // block that has a surface crossing (the sphere case). Catches any
        // out-of-bounds indexing in the face-space → block-space mapping.
        let size = Vector3i::new(8, 8, 8);
        let center = Vector3i::new(4, 4, 4);
        let input = FlatSdfInput::new_sphere(size, center, 3.0);
        let params = BuildRegularMeshParams::default();

        for dir in ALL_DIRECTIONS {
            let mut cache = Cache::default();
            let mut output = MeshArrays::default();
            build_transition_mesh(&input, &params, dir, &mut cache, &mut output);
            // Indices, vertices, normals and lod_data must stay in lock-step.
            assert_eq!(output.vertices.len(), output.normals.len());
            assert_eq!(output.vertices.len(), output.lod_data.len());
            // Every emitted index must reference a real vertex.
            let n = output.vertices.len() as i32;
            for &idx in &output.indices {
                assert!(
                    idx >= 0 && idx < n,
                    "direction {dir}: index {idx} out of range [0,{n})"
                );
            }
        }
    }

    #[test]
    fn too_small_block_is_a_noop() {
        // A block smaller than 3 on any axis must early-out (matches the
        // ZN_ASSERT_RETURN guards in the C++ source).
        let size = Vector3i::new(2, 8, 8);
        let input = FlatSdfInput::new_uniform(size, 1.0);
        let params = BuildRegularMeshParams::default();
        let mut cache = Cache::default();
        let mut output = MeshArrays::default();
        build_transition_mesh(&input, &params, SIDE_NEGATIVE_Z, &mut cache, &mut output);
        assert_eq!(output.indices.len(), 0);
        assert_eq!(output.vertices.len(), 0);
    }

    #[test]
    fn transition_hint_bit_matches_direction() {
        // The per-vertex `transition` byte carries the bit for the face that
        // produced it, so downstream mesh-skinning knows which seam to deform.
        let size = Vector3i::new(10, 10, 10);
        let center = Vector3i::new(5, 5, 5);
        let input = FlatSdfInput::new_sphere(size, center, 4.0);
        let params = BuildRegularMeshParams::default();

        for dir in ALL_DIRECTIONS {
            let mut cache = Cache::default();
            let mut output = MeshArrays::default();
            build_transition_mesh(&input, &params, dir, &mut cache, &mut output);
            let expected_bit = 1u8 << get_face_index(dir);
            for attrib in &output.lod_data {
                assert_ne!(
                    attrib.transition & expected_bit,
                    0,
                    "direction {dir}: vertex transition byte {:?} missing bit {expected_bit}",
                    attrib.transition
                );
            }
        }
    }

    #[test]
    fn face_helpers_match_cpp_layout() {
        // Spot-check the face→block and face-axes mappings against the C++
        // switch bodies. These values are load-bearing for correct sampling
        // and any change here must be deliberate.
        let bs = Vector3i::new(8, 9, 10);

        // SIDE_NEGATIVE_X: face_to_block returns (z, x, y)
        assert_eq!(
            face_to_block(1, 2, 3, SIDE_NEGATIVE_X, bs),
            Vector3i::new(3, 1, 2)
        );
        // SIDE_POSITIVE_X: (bs.x-1-z, y, x)
        assert_eq!(
            face_to_block(1, 2, 3, SIDE_POSITIVE_X, bs),
            Vector3i::new(bs.x - 1 - 3, 2, 1)
        );
        // SIDE_NEGATIVE_Y: (y, z, x)
        assert_eq!(
            face_to_block(1, 2, 3, SIDE_NEGATIVE_Y, bs),
            Vector3i::new(2, 3, 1)
        );
        // SIDE_POSITIVE_Y: (x, bs.y-1-z, y)
        assert_eq!(
            face_to_block(1, 2, 3, SIDE_POSITIVE_Y, bs),
            Vector3i::new(1, bs.y - 1 - 3, 2)
        );
        // SIDE_NEGATIVE_Z: identity (x, y, z)
        assert_eq!(
            face_to_block(1, 2, 3, SIDE_NEGATIVE_Z, bs),
            Vector3i::new(1, 2, 3)
        );
        // SIDE_POSITIVE_Z: (y, x, bs.z-1-z)
        assert_eq!(
            face_to_block(1, 2, 3, SIDE_POSITIVE_Z, bs),
            Vector3i::new(2, 1, bs.z - 1 - 3)
        );

        // Face-axes: must pick the two axes tangent to the face.
        assert_eq!(get_face_axes(SIDE_NEGATIVE_X), (AXIS_Y, AXIS_Z));
        assert_eq!(get_face_axes(SIDE_POSITIVE_X), (AXIS_Z, AXIS_Y));
        assert_eq!(get_face_axes(SIDE_NEGATIVE_Y), (AXIS_Z, AXIS_X));
        assert_eq!(get_face_axes(SIDE_POSITIVE_Y), (AXIS_X, AXIS_Z));
        assert_eq!(get_face_axes(SIDE_NEGATIVE_Z), (AXIS_X, AXIS_Y));
        assert_eq!(get_face_axes(SIDE_POSITIVE_Z), (AXIS_Y, AXIS_X));

        // Face indices: 0..5, one per face.
        assert_eq!(get_face_index(SIDE_NEGATIVE_X), 0);
        assert_eq!(get_face_index(SIDE_POSITIVE_X), 1);
        assert_eq!(get_face_index(SIDE_NEGATIVE_Y), 2);
        assert_eq!(get_face_index(SIDE_POSITIVE_Y), 3);
        assert_eq!(get_face_index(SIDE_NEGATIVE_Z), 4);
        assert_eq!(get_face_index(SIDE_POSITIVE_Z), 5);
    }
}
