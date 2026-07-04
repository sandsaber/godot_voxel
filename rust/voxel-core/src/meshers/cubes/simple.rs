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

                    // One unit quad at (fx, fy, d), expressed in unpadded
                    // block coordinates. `d` already ranges from 0 at the
                    // negative boundary; the other two axes are iterated in
                    // padded voxel coordinates and must be shifted back.
                    let fx0 = fx - PADDING;
                    let fy0 = fy - PADDING;
                    let fx1 = fx0 + 1;
                    let fy1 = fy0 + 1;
                    let mut v = [Vector3f::new(0.0, 0.0, 0.0); 4];
                    v[0][xa] = fx0 as f32;
                    v[0][ya] = fy0 as f32;
                    v[1][xa] = fx1 as f32;
                    v[1][ya] = fy0 as f32;
                    v[2][xa] = fx0 as f32;
                    v[2][ya] = fy1 as f32;
                    v[3][xa] = fx1 as f32;
                    v[3][ya] = fy1 as f32;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn color_from_u32(v: u32) -> Color8 {
        Color8::from_u32(v)
    }

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
    fn single_voxel_positions_are_unpadded() {
        let mut out = [CubesArrays::new(), CubesArrays::new()];
        let solid = Color8::new(255, 255, 255, 255).to_u32();
        let transparent = Color8::new(0, 0, 0, 0).to_u32();
        let voxels = build_buffer(3, 3, 3, |x, y, z| {
            if (x, y, z) == (1, 1, 1) {
                solid
            } else {
                transparent
            }
        });

        build_simple_cubes(&mut out, &voxels, [3, 3, 3], color_from_u32);

        assert_eq!(out[MATERIAL_OPAQUE].triangle_count(), 12);
        assert_eq!(out[MATERIAL_OPAQUE].vertex_count(), 24);
        for p in &out[MATERIAL_OPAQUE].positions {
            assert!(
                (0.0..=1.0).contains(&p.x)
                    && (0.0..=1.0).contains(&p.y)
                    && (0.0..=1.0).contains(&p.z),
                "position should be in the unpadded 1x1x1 cube, got {p:?}"
            );
        }
    }
}
