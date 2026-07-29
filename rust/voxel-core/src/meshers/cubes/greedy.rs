//! Greedy cube meshing — merges coplanar same-color faces into larger quads.
//!
//! Ported from `build_voxel_mesh_as_greedy_cubes` in
//! `meshers/cubes/voxel_mesher_cubes.cpp`. The algorithm sweeps the voxel
//! block along each of the 3 axes, building a 2D "deck" mask of face
//! boundaries, then greedily extends rectangles along X then Y to produce
//! merged quads. Output is split into opaque and transparent material
//! surfaces (based on `color.a < 1.0`).
//!
//! Voxel indexing is ZXY (`index = y + size.y * (x + size.x * z)`, Y
//! innermost) — matching the rest of the engine.

use crate::math::{Color, Color8, Vector3f};
use crate::meshers::cubes::arrays::CubesArrays;

/// Voxel padding required around the block (matches `VoxelMesherCubes::PADDING`).
pub const PADDING: i32 = 1;

/// Number of material slots (opaque + transparent). Matches `MATERIAL_COUNT`.
pub const MATERIAL_COUNT: usize = 2;
const MATERIAL_OPAQUE: usize = 0;
const MATERIAL_TRANSPARENT: usize = 1;

/// For `axis in 0..3`, returns `[x_axis, y_axis]` — the two axes perpendicular
/// to `axis`. Matches `g_face_axes_lut`.
pub(super) const FACE_AXES_LUT: [[usize; 2]; 3] = [
    [1, 2], // axis X → (Y, Z)
    [0, 2], // axis Y → (X, Z)
    [0, 1], // axis Z → (X, Y)
];

/// Triangle indices for a quad, indexed by `[axis][front_or_back]`.
/// Matches `g_indices_lut`. Vertex layout:
/// ```text
/// 2-----3
/// |     |
/// |     |
/// 0-----1
/// ```
pub(super) const INDICES_LUT: [[[u32; 6]; 2]; 3] = [
    // X
    [
        [0, 3, 2, 0, 1, 3], // Front
        [0, 2, 3, 0, 3, 1], // Back
    ],
    // Y
    [
        [0, 2, 3, 0, 3, 1], // Front
        [0, 3, 2, 0, 1, 3], // Back
    ],
    // Z
    [
        [0, 3, 2, 0, 1, 3], // Front
        [0, 2, 3, 0, 3, 1], // Back
    ],
];

const SIDE_NONE: u8 = 2;
const SIDE_FRONT: u8 = 0;
const SIDE_BACK: u8 = 1;

/// Returns 0 for transparent, 1 for partial, 2 for opaque. Matches
/// `get_alpha_index`.
#[inline]
pub(super) fn alpha_index(c: Color8) -> u8 {
    (c.a == 0xff) as u8 + (c.a > 0) as u8
}

/// One cell of the per-deck face mask: the raw voxel value plus which side
/// the face faces. Matches the C++ `MaskValue` struct.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct MaskValue {
    color: u32,
    side: u8,
}

/// Build a greedy cube mesh from a flat ZXY voxel buffer.
///
/// `voxels` is indexed as `y + size.y * (x + size.x * z)`. `color_func`
/// converts a raw voxel value to a [`Color8`]. Output is written into
/// `out[MATERIAL_OPAQUE]` and `out[MATERIAL_TRANSPARENT]`.
///
/// Ported from `build_voxel_mesh_as_greedy_cubes`.
pub fn build_greedy_cubes<F>(
    out: &mut [CubesArrays; MATERIAL_COUNT],
    voxels: &[u32],
    block_size: [i32; 3],
    color_func: F,
) where
    F: Fn(u32) -> Color8,
{
    assert!(
        block_size[0] >= 2 * PADDING
            && block_size[1] >= 2 * PADDING
            && block_size[2] >= 2 * PADDING,
        "block too small for padding"
    );

    let min_pos = [PADDING; 3];
    let max_pos = [
        block_size[0] - PADDING,
        block_size[1] - PADDING,
        block_size[2] - PADDING,
    ];
    let row_size = block_size[1] as usize;
    let deck_size = block_size[0] as usize * row_size;

    // Neighbor offset along each axis in the flat ZXY buffer.
    let neighbor_offset = [
        row_size,                          // X
        1,                                 // Y
        block_size[0] as usize * row_size, // Z
    ];

    let mut index_offsets = [0u32; MATERIAL_COUNT];

    for za in 0..3usize {
        let xa = FACE_AXES_LUT[za][0];
        let ya = FACE_AXES_LUT[za][1];

        let mask_size_x = (max_pos[xa] - min_pos[xa]) as usize;
        let mask_size_y = (max_pos[ya] - min_pos[ya]) as usize;
        let mut mask = vec![MaskValue::default(); mask_size_x * mask_size_y];

        // For each deck (slice perpendicular to `za`).
        let d_start = (min_pos[za] - PADDING) as usize;
        let d_end = max_pos[za] as usize;
        for d in d_start..d_end {
            // Build the mask for this deck.
            for fy in min_pos[ya]..max_pos[ya] {
                for fx in min_pos[xa]..max_pos[xa] {
                    let mut pos = [0usize; 3];
                    pos[xa] = fx as usize;
                    pos[ya] = fy as usize;
                    pos[za] = d;

                    let voxel_index = pos[1] + pos[0] * row_size + pos[2] * deck_size;
                    let raw0 = voxels[voxel_index];
                    let raw1 = voxels[voxel_index + neighbor_offset[za]];

                    let c0 = color_func(raw0);
                    let c1 = color_func(raw1);
                    let ai0 = alpha_index(c0);
                    let ai1 = alpha_index(c1);

                    let mv = if ai0 == ai1 {
                        MaskValue {
                            color: 0,
                            side: SIDE_NONE,
                        }
                    } else if ai0 > ai1 {
                        MaskValue {
                            color: raw0,
                            side: SIDE_BACK,
                        }
                    } else {
                        MaskValue {
                            color: raw1,
                            side: SIDE_FRONT,
                        }
                    };

                    let mx = (fx - PADDING) as usize;
                    let my = (fy - PADDING) as usize;
                    mask[mx + my * mask_size_x] = mv;
                }
            }

            // Greedy quad merging.
            for fy in 0..mask_size_y {
                let mut fx = 0;
                while fx < mask_size_x {
                    let m = mask[fx + fy * mask_size_x];
                    if m.side == SIDE_NONE {
                        fx += 1;
                        continue;
                    }

                    // Extend along X.
                    let mut rx = fx + 1;
                    while rx < mask_size_x && mask[rx + fy * mask_size_x] == m {
                        rx += 1;
                    }

                    // Extend along Y.
                    let mut ry = fy + 1;
                    while ry < mask_size_y && (fx..rx).all(|x| mask[x + ry * mask_size_x] == m) {
                        ry += 1;
                    }

                    // Emit the quad.
                    let color8 = color_func(m.color);
                    let colorf = Color::new(
                        color8.r as f32 / 255.0,
                        color8.g as f32 / 255.0,
                        color8.b as f32 / 255.0,
                        color8.a as f32 / 255.0,
                    );
                    let material_index = if colorf.a < 0.999 {
                        MATERIAL_TRANSPARENT
                    } else {
                        MATERIAL_OPAQUE
                    };
                    let arrays = &mut out[material_index];

                    // Four corners in the (xa, ya) plane at depth d.
                    let mut v = [Vector3f::new(0.0, 0.0, 0.0); 4];
                    v[0][xa] = fx as f32;
                    v[0][ya] = fy as f32;
                    v[1][xa] = rx as f32;
                    v[1][ya] = fy as f32;
                    v[2][xa] = fx as f32;
                    v[2][ya] = ry as f32;
                    v[3][xa] = rx as f32;
                    v[3][ya] = ry as f32;
                    for vi in &mut v {
                        vi[za] = d as f32;
                    }

                    let mut n = Vector3f::new(0.0, 0.0, 0.0);
                    n[za] = if m.side == SIDE_FRONT { -1.0 } else { 1.0 };

                    let base = index_offsets[material_index] as i32;
                    for vi in &v {
                        arrays.positions.push(*vi);
                        arrays.colors.push(colorf);
                        arrays.normals.push(n);
                    }
                    let lut = INDICES_LUT[za][m.side as usize];
                    for &li in &lut {
                        arrays.indices.push(base + li as i32);
                    }
                    index_offsets[material_index] += 4;

                    // Mark consumed cells.
                    for j in fy..ry {
                        for i in fx..rx {
                            mask[i + j * mask_size_x].side = SIDE_NONE;
                        }
                    }

                    fx = rx;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::manual_range_contains)]
mod tests {
    use super::*;

    /// Identity color func: treats the raw u32 as a packed RGBA Color8.
    fn color_from_u32(v: u32) -> Color8 {
        Color8::from_u32(v)
    }

    /// Build a flat ZXY voxel buffer of the given size from a closure.
    fn build_buffer<F: FnMut(usize, usize, usize) -> u32>(
        sx: usize,
        sy: usize,
        sz: usize,
        mut f: F,
    ) -> Vec<u32> {
        let mut v = vec![0u32; sx * sy * sz];
        for z in 0..sz {
            for x in 0..sx {
                for y in 0..sy {
                    v[y + x * sy + z * sx * sy] = f(x, y, z);
                }
            }
        }
        v
    }

    #[test]
    fn empty_block_produces_no_faces() {
        let mut out = [CubesArrays::new(), CubesArrays::new()];
        // All-transparent voxels (alpha=0) → no face boundaries.
        let voxels = build_buffer(4, 4, 4, |_, _, _| Color8::new(0, 0, 0, 0).to_u32());
        build_greedy_cubes(&mut out, &voxels, [4, 4, 4], color_from_u32);
        assert_eq!(out[0].triangle_count() + out[1].triangle_count(), 0);
    }

    #[test]
    fn solid_uniform_block_produces_only_outer_faces() {
        // A 2×2×2 solid region inside a 4×4×4 block with transparent padding.
        // Faces appear only at the solid/transparent boundary (the cube's 6
        // sides), each merged into one quad by the greedy pass.
        let mut out = [CubesArrays::new(), CubesArrays::new()];
        let voxels = build_buffer(4, 4, 4, |x, y, z| {
            if x >= 1 && x < 3 && y >= 1 && y < 3 && z >= 1 && z < 3 {
                Color8::new(255, 255, 255, 255).to_u32()
            } else {
                Color8::new(0, 0, 0, 0).to_u32()
            }
        });
        build_greedy_cubes(&mut out, &voxels, [4, 4, 4], color_from_u32);
        // 6 faces, each one merged quad (2 triangles).
        let total_tris = out[0].triangle_count() + out[1].triangle_count();
        assert_eq!(
            total_tris, 12,
            "expected 6 quads (12 tris), got {total_tris}"
        );
    }

    #[test]
    fn greedy_merges_coplanar_faces() {
        // A 2x2x2 solid region inside a 4x4x4 block (with padding) should
        // merge into one quad per face, not four.
        let mut out = [CubesArrays::new(), CubesArrays::new()];
        let voxels = build_buffer(4, 4, 4, |x, y, z| {
            // Solid in the [1..3) cube (the padded interior).
            if x >= 1 && x < 3 && y >= 1 && y < 3 && z >= 1 && z < 3 {
                Color8::new(255, 0, 0, 255).to_u32()
            } else {
                Color8::new(0, 0, 0, 0).to_u32()
            }
        });
        build_greedy_cubes(&mut out, &voxels, [4, 4, 4], color_from_u32);
        // 6 faces, each a single merged quad (2 triangles).
        let total_tris = out[0].triangle_count() + out[1].triangle_count();
        assert_eq!(
            total_tris, 12,
            "greedy should merge into 6 quads, got {total_tris} tris"
        );
        // Each face is one quad → 4 verts per face × 6 = 24 verts.
        let total_verts = out[0].vertex_count() + out[1].vertex_count();
        assert_eq!(total_verts, 24);
    }

    #[test]
    fn transparent_voxels_route_to_transparent_material() {
        let mut out = [CubesArrays::new(), CubesArrays::new()];
        // Half-alpha solid → transparent material.
        let voxels = build_buffer(4, 4, 4, |x, y, z| {
            if x >= 1 && x < 3 && y >= 1 && y < 3 && z >= 1 && z < 3 {
                Color8::new(255, 0, 0, 128).to_u32() // partial alpha
            } else {
                Color8::new(0, 0, 0, 0).to_u32()
            }
        });
        build_greedy_cubes(&mut out, &voxels, [4, 4, 4], color_from_u32);
        assert!(out[MATERIAL_TRANSPARENT].triangle_count() > 0);
        assert_eq!(out[MATERIAL_OPAQUE].triangle_count(), 0);
    }

    #[test]
    fn alpha_index_classifies_three_states() {
        assert_eq!(alpha_index(Color8::new(0, 0, 0, 0)), 0); // transparent
        assert_eq!(alpha_index(Color8::new(0, 0, 0, 128)), 1); // partial
        assert_eq!(alpha_index(Color8::new(0, 0, 0, 255)), 2); // opaque
    }

    #[test]
    fn face_axes_lut_pairs_perpendicular_axes() {
        for (axis, &[xa, ya]) in FACE_AXES_LUT.iter().enumerate() {
            assert_ne!(xa, axis);
            assert_ne!(ya, axis);
            assert_ne!(xa, ya);
        }
    }
}
