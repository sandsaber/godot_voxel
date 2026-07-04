//! Simple (non-greedy) cube meshing — one quad per voxel face.
//!
//! Ported from `build_voxel_mesh_as_simple_cubes` in
//! `meshers/cubes/voxel_mesher_cubes.cpp`. The same face-culling logic as
//! [`super::greedy`] but without rectangle merging: every boundary voxel-face
//! becomes its own quad. Useful as a correctness reference and for blocks too
//! small to benefit from greedy merging.

use crate::math::{Color, Color8, Vector3f};
use crate::meshers::cubes::arrays::CubesArrays;
use crate::meshers::cubes::greedy::{
    alpha_index, FACE_AXES_LUT, INDICES_LUT, MATERIAL_COUNT, PADDING,
};

const MATERIAL_OPAQUE: usize = 0;
const MATERIAL_TRANSPARENT: usize = 1;
const SIDE_FRONT: u8 = 0;
const SIDE_BACK: u8 = 1;

/// Build a non-greedy cube mesh. Same contract as
/// [`super::greedy::build_greedy_cubes`]. Ported from
/// `build_voxel_mesh_as_simple_cubes`.
pub fn build_simple_cubes<F>(
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
    let neighbor_offset = [row_size, 1, block_size[0] as usize * row_size];
    let mut index_offsets = [0u32; MATERIAL_COUNT];

    for za in 0..3usize {
        let xa = FACE_AXES_LUT[za][0];
        let ya = FACE_AXES_LUT[za][1];

        let d_start = (min_pos[za] - PADDING) as usize;
        let d_end = max_pos[za] as usize;
        for d in d_start..d_end {
            for fy in min_pos[ya]..max_pos[ya] {
                for fx in min_pos[xa]..max_pos[xa] {
                    let mut pos = [0usize; 3];
                    pos[xa] = fx as usize;
                    pos[ya] = fy as usize;
                    pos[za] = d;

                    let vi = pos[1] + pos[0] * row_size + pos[2] * deck_size;
                    let raw0 = voxels[vi];
                    let raw1 = voxels[vi + neighbor_offset[za]];
                    let c0 = color_func(raw0);
                    let c1 = color_func(raw1);
                    let ai0 = alpha_index(c0);
                    let ai1 = alpha_index(c1);

                    if ai0 == ai1 {
                        continue;
                    }
                    let (raw, side) = if ai0 > ai1 {
                        (raw0, SIDE_BACK)
                    } else {
                        (raw1, SIDE_FRONT)
                    };

                    let color8 = color_func(raw);
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

                    // One unit quad at (fx, fy, d).
                    let mut v = [Vector3f::new(0.0, 0.0, 0.0); 4];
                    v[0][xa] = fx as f32;
                    v[0][ya] = fy as f32;
                    v[1][xa] = fx as f32 + 1.0;
                    v[1][ya] = fy as f32;
                    v[2][xa] = fx as f32;
                    v[2][ya] = fy as f32 + 1.0;
                    v[3][xa] = fx as f32 + 1.0;
                    v[3][ya] = fy as f32 + 1.0;
                    for vi in &mut v {
                        vi[za] = d as f32;
                    }
                    let mut n = Vector3f::new(0.0, 0.0, 0.0);
                    n[za] = if side == SIDE_FRONT { -1.0 } else { 1.0 };

                    let base = index_offsets[material_index] as i32;
                    for vi in &v {
                        arrays.positions.push(*vi);
                        arrays.colors.push(colorf);
                        arrays.normals.push(n);
                    }
                    let lut = INDICES_LUT[za][side as usize];
                    for &li in &lut {
                        arrays.indices.push(base + li as i32);
                    }
                    index_offsets[material_index] += 4;
                }
            }
        }
    }
}
