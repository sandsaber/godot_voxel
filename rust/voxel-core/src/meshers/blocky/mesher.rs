//! Blocky meshing core: `BlockyArrays`, face-visibility helpers, and the
//! `generate_mesh` template instantiated over voxel-type.
//!
//! Ported from `meshers/blocky/voxel_mesher_blocky.{h,cpp}` — the visibility
//! helpers (lines ~133–166 of the header) and the `generate_mesh` template
//! (lines ~41–480 of the .cpp). The Godot `Resource`/editor layer (`build`,
//! `_bind_methods`, `VoxelMesherBlocky`) is intentionally NOT ported (Phase 5).
//!
//! ## Indexing convention
//! ZXY with Y innermost: `index = y + sy*(x + sx*z)`, where `s*` are the block
//! dimensions. This matches the cubes mesher and the C++ `generate_mesh`.
//!
//! ## Data layout
//! The block must be padded by 1 voxel on every side (`PADDING = 1`); the mesher
//! iterates the interior `[1, size-PADDING)` and reads neighbors directly
//! without bounds checks (the padding guarantees they exist).
//
// Index-based `for` loops and element-wise slice copies mirror the C++ source
// 1:1 to make the parity port easy to diff. Clippy's iterator/copy_from_slice
// suggestions would obscure that correspondence, so they're silenced here.
#![allow(
    clippy::needless_range_loop,
    clippy::manual_memcpy,
    clippy::collapsible_if
)]

use crate::constants::cube_tables::{
    Corner, Edge, Side, CORNER_POSITION, EDGE_CORNERS, OPPOSITE_SIDE, SIDE_CORNERS, SIDE_EDGES,
    SIDE_NORMALS,
};
use crate::math::conv::vec3i_to_vec3f;
use crate::math::{Color, Vector2f, Vector3f, Vector3i};
use crate::meshers::blocky::baked_library::{BakedLibrary, BakedModel, AIR_ID};
use crate::meshers::blocky::{ModelSurface, SideSurface, MAX_SURFACES};

/// Padding (in voxels) the mesher requires on every side of the block.
pub const PADDING: i32 = 1;

/// Default AO darkness used by the C++ `Parameters` struct (`0.8`).
pub const DEFAULT_BAKED_OCCLUSION_DARKNESS: f32 = 0.8;

/// Per-material mesh output of the blocky mesher. Parallel `Vec`s of vertex
/// attributes + an index buffer (mirrors `CubesArrays`, plus tangents to match
/// the C++ `VoxelMesherBlocky::Arrays`).
#[derive(Debug, Default, Clone)]
pub struct BlockyArrays {
    pub positions: Vec<Vector3f>,
    pub normals: Vec<Vector3f>,
    pub uvs: Vec<Vector2f>,
    pub colors: Vec<Color>,
    pub indices: Vec<i32>,
    pub tangents: Vec<f32>,
}

impl BlockyArrays {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        self.colors.clear();
        self.indices.clear();
        self.tangents.clear();
    }

    /// Vertex count (from the positions buffer).
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }
}

// ---------------------------------------------------------------------------
// Visibility helpers (header `voxel_mesher_blocky.h`, lines ~133–166)
// ---------------------------------------------------------------------------

/// `contributes_to_ao` — whether voxel `voxel_id`'s neighbors count it as an
/// AO occluder. Out-of-range ids are treated as AO contributors (matches C++).
#[inline]
pub fn contributes_to_ao(lib: &BakedLibrary, voxel_id: u32) -> bool {
    if (voxel_id as usize) < lib.models.len() {
        lib.models[voxel_id as usize].contributes_to_ao
    } else {
        true
    }
}

/// `is_face_visible_regardless_of_shape` — visibility that depends only on the
/// neighbor's metadata, not its baked shape.
#[inline]
pub fn is_face_visible_regardless_of_shape(vt: &BakedModel, other_vt: &BakedModel) -> bool {
    // TODO Maybe we could get rid of `empty` here and instead set
    // `culls_neighbors` to false during baking.
    other_vt.empty
        || (other_vt.transparency_index > vt.transparency_index)
        || !other_vt.culls_neighbors
}

/// `is_face_visible_according_to_shape` — shape-based visibility using the
/// side-pattern occlusion matrix. Does not account for the "regardless of
/// shape" factors.
#[inline]
pub fn is_face_visible_according_to_shape(
    lib: &BakedLibrary,
    vt: &BakedModel,
    other_vt: &BakedModel,
    side: i32,
) -> bool {
    let ai = vt.model.side_pattern_indices[side as usize];
    let bi = other_vt.model.side_pattern_indices[OPPOSITE_SIDE[side as usize] as usize];
    // Patterns are not the same, and B does not occlude A.
    (ai != bi) && !lib.get_side_pattern_occlusion(bi, ai)
}

/// `is_face_visible` — combine both visibility checks. Returns `true` if the
/// face on `side` of `vt` is visible against neighbor `other_voxel_id`.
#[inline]
pub fn is_face_visible(
    lib: &BakedLibrary,
    vt: &BakedModel,
    other_voxel_id: u32,
    side: i32,
) -> bool {
    if (other_voxel_id as usize) < lib.models.len() {
        let other_vt = &lib.models[other_voxel_id as usize];
        if is_face_visible_regardless_of_shape(vt, other_vt) {
            true
        } else {
            is_face_visible_according_to_shape(lib, vt, other_vt, side)
        }
    } else {
        // Invalid voxels are treated like air.
        true
    }
}

// ---------------------------------------------------------------------------
// AO helpers
// ---------------------------------------------------------------------------

/// Component-wise color multiply (the C++ `Color::operator*`). Used for
/// `Color(gs,gs,gs) * modulate_color` in the AO path.
#[inline]
fn color_mul(a: Color, b: Color) -> Color {
    Color::new(a.r * b.r, a.g * b.g, a.b * b.b, a.a * b.a)
}

/// Compute the 8-entry `shaded_corner` array for one face. Ported verbatim
/// from the `bake_occlusion` block in `generate_mesh`. `read_voxel` returns the
/// voxel id at a linear buffer index (so the helper is generic over `T`).
fn compute_shaded_corner<F>(
    side: usize,
    library: &BakedLibrary,
    edge_neighbor_lut: &[i32; Edge::COUNT],
    corner_neighbor_lut: &[i32; Corner::COUNT],
    read_voxel: F,
) -> [u8; 8]
where
    F: Fn(i32) -> u16,
{
    let mut shaded_corner = [0u8; 8];

    // Edges first: each edge neighbor increments its two corners.
    for j in 0..4usize {
        let edge = SIDE_EDGES[side][j];
        let edge_neighbor_id = read_voxel(edge_neighbor_lut[edge]);
        if contributes_to_ao(library, edge_neighbor_id as u32) {
            let c0 = EDGE_CORNERS[edge][0];
            let c1 = EDGE_CORNERS[edge][1];
            shaded_corner[c0] += 1;
            shaded_corner[c1] += 1;
        }
    }
    // Corners: if two edges already cover this corner, treat as fully shaded
    // (3); otherwise add the corner neighbor.
    for j in 0..4usize {
        let corner = SIDE_CORNERS[side][j];
        if shaded_corner[corner] == 2 {
            shaded_corner[corner] = 3;
        } else {
            let corner_neighbor_id = read_voxel(corner_neighbor_lut[corner]);
            if contributes_to_ao(library, corner_neighbor_id as u32) {
                shaded_corner[corner] += 1;
            }
        }
    }

    shaded_corner
}

// ---------------------------------------------------------------------------
// generate_mesh
// ---------------------------------------------------------------------------

/// Build neighbor-offset lookup tables (in linear-index units) from the block
/// dimensions. Matches the `*_neighbor_lut` construction at the top of the C++
/// `generate_mesh`.
fn build_neighbor_luts(
    row_size: i32,
    deck_size: i32,
    side_neighbor_lut: &mut [i32; Side::COUNT],
    edge_neighbor_lut: &mut [i32; Edge::COUNT],
    corner_neighbor_lut: &mut [i32; Corner::COUNT],
) {
    // The C++ Side enum used here matches our `Side` discriminants:
    //   Left=0 (+X), Right=1 (-X), Bottom=2 (-Y), Top=3 (+Y), Back=4 (-Z), Front=5 (+Z)
    side_neighbor_lut[Side::Left as usize] = row_size; // +X
    side_neighbor_lut[Side::Right as usize] = -row_size; // -X
    side_neighbor_lut[Side::Back as usize] = -deck_size; // -Z
    side_neighbor_lut[Side::Front as usize] = deck_size; // +Z
    side_neighbor_lut[Side::Bottom as usize] = -1; // -Y
    side_neighbor_lut[Side::Top as usize] = 1; // +Y

    // Edges are sums of the two incident side offsets.
    edge_neighbor_lut[Edge::BottomBack as usize] =
        side_neighbor_lut[Side::Bottom as usize] + side_neighbor_lut[Side::Back as usize];
    edge_neighbor_lut[Edge::BottomFront as usize] =
        side_neighbor_lut[Side::Bottom as usize] + side_neighbor_lut[Side::Front as usize];
    edge_neighbor_lut[Edge::BottomLeft as usize] =
        side_neighbor_lut[Side::Bottom as usize] + side_neighbor_lut[Side::Left as usize];
    edge_neighbor_lut[Edge::BottomRight as usize] =
        side_neighbor_lut[Side::Bottom as usize] + side_neighbor_lut[Side::Right as usize];
    edge_neighbor_lut[Edge::BackLeft as usize] =
        side_neighbor_lut[Side::Back as usize] + side_neighbor_lut[Side::Left as usize];
    edge_neighbor_lut[Edge::BackRight as usize] =
        side_neighbor_lut[Side::Back as usize] + side_neighbor_lut[Side::Right as usize];
    edge_neighbor_lut[Edge::FrontLeft as usize] =
        side_neighbor_lut[Side::Front as usize] + side_neighbor_lut[Side::Left as usize];
    edge_neighbor_lut[Edge::FrontRight as usize] =
        side_neighbor_lut[Side::Front as usize] + side_neighbor_lut[Side::Right as usize];
    edge_neighbor_lut[Edge::TopBack as usize] =
        side_neighbor_lut[Side::Top as usize] + side_neighbor_lut[Side::Back as usize];
    edge_neighbor_lut[Edge::TopFront as usize] =
        side_neighbor_lut[Side::Top as usize] + side_neighbor_lut[Side::Front as usize];
    edge_neighbor_lut[Edge::TopLeft as usize] =
        side_neighbor_lut[Side::Top as usize] + side_neighbor_lut[Side::Left as usize];
    edge_neighbor_lut[Edge::TopRight as usize] =
        side_neighbor_lut[Side::Top as usize] + side_neighbor_lut[Side::Right as usize];

    // Corners are sums of the three incident side offsets.
    corner_neighbor_lut[Corner::BottomBackLeft as usize] = side_neighbor_lut[Side::Bottom as usize]
        + side_neighbor_lut[Side::Back as usize]
        + side_neighbor_lut[Side::Left as usize];
    corner_neighbor_lut[Corner::BottomBackRight as usize] = side_neighbor_lut
        [Side::Bottom as usize]
        + side_neighbor_lut[Side::Back as usize]
        + side_neighbor_lut[Side::Right as usize];
    corner_neighbor_lut[Corner::BottomFrontRight as usize] = side_neighbor_lut
        [Side::Bottom as usize]
        + side_neighbor_lut[Side::Front as usize]
        + side_neighbor_lut[Side::Right as usize];
    corner_neighbor_lut[Corner::BottomFrontLeft as usize] = side_neighbor_lut
        [Side::Bottom as usize]
        + side_neighbor_lut[Side::Front as usize]
        + side_neighbor_lut[Side::Left as usize];
    corner_neighbor_lut[Corner::TopBackLeft as usize] = side_neighbor_lut[Side::Top as usize]
        + side_neighbor_lut[Side::Back as usize]
        + side_neighbor_lut[Side::Left as usize];
    corner_neighbor_lut[Corner::TopBackRight as usize] = side_neighbor_lut[Side::Top as usize]
        + side_neighbor_lut[Side::Back as usize]
        + side_neighbor_lut[Side::Right as usize];
    corner_neighbor_lut[Corner::TopFrontRight as usize] = side_neighbor_lut[Side::Top as usize]
        + side_neighbor_lut[Side::Front as usize]
        + side_neighbor_lut[Side::Right as usize];
    corner_neighbor_lut[Corner::TopFrontLeft as usize] = side_neighbor_lut[Side::Top as usize]
        + side_neighbor_lut[Side::Front as usize]
        + side_neighbor_lut[Side::Left as usize];
}

/// The core blocky meshing algorithm. Ported from the C++ `generate_mesh<T>`
/// template (fluids, collision, and tinting are omitted; this port emits baked
/// side + inside geometry with face culling and optional AO).
///
/// * `out` — one [`BlockyArrays`] per material index (sized to
///   `library.indexed_materials_count`, or at least 1).
/// * `type_buffer` — flat voxel-id channel, ZXY layout, padded by [`PADDING`]
///   on every side.
/// * `block_size` — full (padded) block dimensions.
/// * `bake_occlusion` — if true, darkens vertices based on neighbor occupancy
///   (the 0fps-style corner AO).
/// * `baked_occlusion_darkness` — AO strength (C++ default `0.8`).
pub fn generate_mesh<T>(
    out: &mut [BlockyArrays],
    type_buffer: &[T],
    block_size: Vector3i,
    library: &BakedLibrary,
    bake_occlusion: bool,
    baked_occlusion_darkness: f32,
) where
    T: Copy + Into<u16>,
{
    assert!(
        block_size.x >= 2 * PADDING && block_size.y >= 2 * PADDING && block_size.z >= 2 * PADDING,
        "block too small for padding"
    );

    let row_size = block_size.y;
    let deck_size = block_size.x * row_size;

    // Data must be padded, hence the off-by-one.
    let min = Vector3i::new(PADDING, PADDING, PADDING);
    let max = block_size - Vector3i::new(PADDING, PADDING, PADDING);

    // Per-material running vertex count (for index rebasing).
    let mut index_offsets = vec![0i32; out.len()];

    let mut side_neighbor_lut = [0i32; Side::COUNT];
    let mut edge_neighbor_lut = [0i32; Edge::COUNT];
    let mut corner_neighbor_lut = [0i32; Corner::COUNT];
    build_neighbor_luts(
        row_size,
        deck_size,
        &mut side_neighbor_lut,
        &mut edge_neighbor_lut,
        &mut corner_neighbor_lut,
    );

    // ZXY iteration, Y innermost.
    for z in min.z..max.z {
        for x in min.x..max.x {
            for y in min.y..max.y {
                let voxel_index = (y + x * row_size + z * deck_size) as usize;
                let voxel_id: u16 = type_buffer[voxel_index].into();

                // Air / unknown voxels produce nothing.
                if voxel_id == AIR_ID || !library.has_model(voxel_id as u32) {
                    continue;
                }

                let voxel = &library.models[voxel_id as usize];

                // Fluids are not handled in this port (Phase 5). Skip them to
                // avoid emitting raw side geometry that assumes fluid shaping.
                if voxel.fluid_index != crate::meshers::blocky::NULL_FLUID_INDEX {
                    continue;
                }

                let model = &voxel.model;

                // ---- Visibility of sides ----
                let mut visible_sides_mask: u32 = 0;
                for side in 0..Side::COUNT as u32 {
                    if (model.empty_sides_mask & (1 << side)) != 0 {
                        // This side is empty.
                        continue;
                    }

                    let neighbor_voxel_id: u16 = type_buffer
                        [(voxel_index as i32 + side_neighbor_lut[side as usize]) as usize]
                        .into();

                    // Invalid voxels are treated like air.
                    if (neighbor_voxel_id as usize) < library.models.len() {
                        let other_vt = &library.models[neighbor_voxel_id as usize];
                        if !is_face_visible_regardless_of_shape(voxel, other_vt)
                            && !is_face_visible_according_to_shape(
                                library,
                                voxel,
                                other_vt,
                                side as i32,
                            )
                        {
                            // Completely occluded.
                            continue;
                        }
                    }

                    visible_sides_mask |= 1 << side;
                }

                let model_surface_count = model.surface_count as usize;
                let model_surfaces: &[ModelSurface; crate::meshers::blocky::MAX_SURFACES] =
                    &model.surfaces;
                let modulate_color = voxel.color;

                // ---- Sides ----
                for side in 0..Side::COUNT {
                    if (visible_sides_mask & (1 << side)) == 0 {
                        // Culled.
                        continue;
                    }

                    // By default render the whole side.
                    let mut side_surfaces_ref: &[SideSurface; MAX_SURFACES] =
                        &model.sides_surfaces[side];

                    // Cutout lookup (only if the model opted in).
                    #[allow(unused_assignments)]
                    let mut cut_surfaces_storage: Option<
                        [SideSurface; MAX_SURFACES],
                    > = None;
                    if voxel.cutout_sides_enabled {
                        let neighbor_voxel_id: u16 = type_buffer
                            [(voxel_index as i32 + side_neighbor_lut[side]) as usize]
                            .into();
                        if (neighbor_voxel_id as usize) < library.models.len() {
                            let other_vt = &library.models[neighbor_voxel_id as usize];
                            let neighbor_shape_id =
                                other_vt.model.side_pattern_indices[OPPOSITE_SIDE[side] as usize];

                            if let Some(entries) = voxel
                                .cutout_side_surfaces
                                .get(&(side as u8, neighbor_shape_id))
                            {
                                // Use pre-cut side instead.
                                let mut arr: [SideSurface; MAX_SURFACES] = Default::default();
                                for (k, s) in entries.iter().take(MAX_SURFACES).enumerate() {
                                    arr[k] = s.clone();
                                }
                                cut_surfaces_storage = Some(arr);
                                side_surfaces_ref = cut_surfaces_storage.as_ref().unwrap();
                            }
                        }
                    }

                    // AO: build the per-corner shade array for this side.
                    let shaded_corner = if bake_occlusion {
                        compute_shaded_corner(
                            side,
                            library,
                            &edge_neighbor_lut,
                            &corner_neighbor_lut,
                            |offset: i32| -> u16 {
                                type_buffer[(voxel_index as i32 + offset) as usize].into()
                            },
                        )
                    } else {
                        [0u8; 8]
                    };

                    // Subtracting 1 because the data is padded.
                    let pos = Vector3f::new((x - 1) as f32, (y - 1) as f32, (z - 1) as f32);

                    for surface_index in 0..model_surface_count {
                        let surface = &model_surfaces[surface_index];
                        let material_id = surface.material_id as usize;
                        if material_id >= out.len() {
                            continue;
                        }
                        let arrays = &mut out[material_id];
                        let index_offset = &mut index_offsets[material_id];

                        let side_surface = &side_surfaces_ref[surface_index];
                        let side_positions = &side_surface.positions;
                        let vertex_count = side_positions.len();
                        let side_uvs = &side_surface.uvs;
                        let side_tangents = &side_surface.tangents;
                        let side_indices = &side_surface.indices;
                        let index_count = side_indices.len();

                        // Positions.
                        let append_index = arrays.positions.len();
                        arrays
                            .positions
                            .resize(append_index + vertex_count, Vector3f::new(0.0, 0.0, 0.0));
                        for i in 0..vertex_count {
                            arrays.positions[append_index + i] = side_positions[i] + pos;
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
                        let n = vec3i_to_vec3f(SIDE_NORMALS[side]);
                        for i in 0..vertex_count {
                            arrays.normals[n_append + i] = n;
                        }

                        // Colors (with optional AO).
                        let c_append = arrays.colors.len();
                        arrays.colors.resize(c_append + vertex_count, Color::WHITE);
                        if bake_occlusion {
                            for i in 0..vertex_count {
                                let vertex_pos = side_positions[i];
                                let mut shade = 0.0f32;
                                for j in 0..4usize {
                                    let corner = SIDE_CORNERS[side][j];
                                    if shaded_corner[corner] != 0 {
                                        let s =
                                            baked_occlusion_darkness * shaded_corner[corner] as f32;
                                        // k = 1 - distance_squared(corner_pos, vertex_pos).
                                        let dx = CORNER_POSITION[corner].x - vertex_pos.x;
                                        let dy = CORNER_POSITION[corner].y - vertex_pos.y;
                                        let dz = CORNER_POSITION[corner].z - vertex_pos.z;
                                        let mut k = 1.0 - (dx * dx + dy * dy + dz * dz);
                                        if k < 0.0 {
                                            k = 0.0;
                                        }
                                        let s = s * k;
                                        if s > shade {
                                            shade = s;
                                        }
                                    }
                                }
                                let gs = 1.0 - shade;
                                arrays.colors[c_append + i] =
                                    color_mul(Color::new(gs, gs, gs, 1.0), modulate_color);
                            }
                        } else {
                            for i in 0..vertex_count {
                                arrays.colors[c_append + i] = modulate_color;
                            }
                        }

                        // Indices.
                        let idx_append = arrays.indices.len();
                        arrays.indices.resize(idx_append + index_count, 0);
                        for j in 0..index_count {
                            arrays.indices[idx_append + j] = *index_offset + side_indices[j];
                        }

                        *index_offset += vertex_count as i32;
                    }
                }

                // ---- Inside (non-side) surfaces ----
                for surface_index in 0..model_surface_count {
                    let surface = &model_surfaces[surface_index];
                    if surface.positions.is_empty() {
                        continue;
                    }
                    let material_id = surface.material_id as usize;
                    if material_id >= out.len() {
                        continue;
                    }
                    let arrays = &mut out[material_id];
                    let index_offset = &mut index_offsets[material_id];

                    let positions = &surface.positions;
                    let vertex_count = positions.len();
                    let normals = &surface.normals;
                    let uvs = &surface.uvs;
                    let tangents = &surface.tangents;
                    let indices = &surface.indices;

                    let pos = Vector3f::new((x - 1) as f32, (y - 1) as f32, (z - 1) as f32);

                    if !tangents.is_empty() {
                        let t_append = arrays.tangents.len();
                        arrays.tangents.resize(t_append + vertex_count * 4, 0.0);
                        for i in 0..vertex_count * 4 {
                            arrays.tangents[t_append + i] = tangents[i];
                        }
                    }

                    for i in 0..vertex_count {
                        arrays.normals.push(normals[i]);
                        arrays.uvs.push(uvs[i]);
                        arrays.positions.push(positions[i] + pos);
                        // TODO handle ambient occlusion on inner parts.
                        arrays.colors.push(modulate_color);
                    }

                    for &idx in indices {
                        arrays.indices.push(*index_offset + idx);
                    }

                    *index_offset += vertex_count as i32;
                }
            }
        }
    }
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

    /// Build a full unit-cube side surface for `side`.
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

    /// A baked library: index 0 = air, index 1 = full opaque cube (one
    /// surface, material 0). `bake_library` is assumed already run by callers
    /// that need occlusion info.
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
    fn visibility_helpers_basic() {
        let lib = full_cube_library_baked();
        let cube = &lib.models[1];
        let air = &lib.models[0];

        // Air is visible regardless of shape.
        assert!(is_face_visible_regardless_of_shape(cube, air));
        // Cube-vs-cube is not visible regardless of shape.
        assert!(!is_face_visible_regardless_of_shape(cube, cube));
        // is_face_visible: cube next to air → visible.
        assert!(is_face_visible(&lib, cube, 0, Side::Front as i32));
    }

    #[test]
    fn contributes_to_ao_respects_flag_and_range() {
        let lib = full_cube_library_baked();
        // Cube contributes (it baked to a full pattern == the full-side pattern,
        // so contributes_to_ao stays true).
        assert!(contributes_to_ao(&lib, 1));
        // Air's pattern is the empty pattern (≠ full pattern), so the bake sets
        // contributes_to_ao = false. Matches C++ `generate_side_culling_matrix`.
        assert!(!contributes_to_ao(&lib, 0));
        // Out-of-range ids are treated as AO contributors.
        assert!(contributes_to_ao(&lib, 9999));
    }

    #[test]
    fn generate_mesh_single_solid_block_emits_all_six_faces() {
        // 3x3x3 block (1 padding on each side, 1 interior voxel).
        // Layout: air everywhere except the center = cube (id 1).
        let size = Vector3i::new(3, 3, 3);
        let mut buf = vec![0u16; (size.x * size.y * size.z) as usize];
        let center = 1 + size.y + size.x * size.y; // (x=1,z=1) → y + x*sy + z*sx*sy
        buf[center as usize] = 1;

        let lib = full_cube_library_baked();
        let mut out = vec![BlockyArrays::new()];

        generate_mesh(
            &mut out,
            &buf,
            size,
            &lib,
            false, // no AO
            DEFAULT_BAKED_OCCLUSION_DARKNESS,
        );

        let m = &out[0];
        // 6 faces × 4 verts = 24 vertices.
        assert_eq!(m.vertex_count(), 24);
        // 6 faces × 6 indices = 36 indices.
        assert_eq!(m.indices.len(), 36);
        // 12 triangles.
        assert_eq!(m.indices.len() / 3, 12);
    }

    #[test]
    fn generate_mesh_two_adjacent_cubes_cull_shared_face() {
        // 4x3x3 block: two cubes side by side along X at the interior.
        let size = Vector3i::new(4, 3, 3);
        let mut buf = vec![0u16; (size.x * size.y * size.z) as usize];
        let row = size.y;
        let deck = size.x * row;
        let idx = |x: i32, y: i32, z: i32| -> usize { (y + x * row + z * deck) as usize };
        buf[idx(1, 1, 1)] = 1;
        buf[idx(2, 1, 1)] = 1;

        let lib = full_cube_library_baked();
        let mut out = vec![BlockyArrays::new()];
        generate_mesh(
            &mut out,
            &buf,
            size,
            &lib,
            false,
            DEFAULT_BAKED_OCCLUSION_DARKNESS,
        );

        // Two isolated cubes = 12 faces. Sharing one pair culls 2 faces → 10.
        let m = &out[0];
        assert_eq!(m.vertex_count(), 10 * 4);
        assert_eq!(m.indices.len(), 10 * 6);
    }

    #[test]
    fn generate_mesh_ao_darkens_occluded_corners() {
        // A 3x3x3 "corner pocket": center cube surrounded so two of its edges
        // on the top face are occluded. We place occluders along +X and +Z of
        // the center on the same Y, and one on top.
        let size = Vector3i::new(5, 5, 5);
        let mut buf = vec![0u16; (size.x * size.y * size.z) as usize];
        let row = size.y;
        let deck = size.x * row;
        let idx = |x: i32, y: i32, z: i32| -> usize { (y + x * row + z * deck) as usize };
        // Center block.
        buf[idx(2, 2, 2)] = 1;
        // Occluders around it (these will darken AO on the center's faces).
        buf[idx(3, 2, 2)] = 1; // +X neighbor
        buf[idx(2, 2, 3)] = 1; // +Z neighbor

        let lib = full_cube_library_baked();

        // With AO off → all colors are white.
        let mut out_no_ao = vec![BlockyArrays::new()];
        generate_mesh(
            &mut out_no_ao,
            &buf,
            size,
            &lib,
            false,
            DEFAULT_BAKED_OCCLUSION_DARKNESS,
        );
        for c in &out_no_ao[0].colors {
            assert_eq!(*c, Color::WHITE, "AO disabled → white");
        }

        // With AO on → at least one color is darker than white.
        let mut out_ao = vec![BlockyArrays::new()];
        generate_mesh(
            &mut out_ao,
            &buf,
            size,
            &lib,
            true,
            DEFAULT_BAKED_OCCLUSION_DARKNESS,
        );
        let any_darker = out_ao[0].colors.iter().any(|c| c.r < 0.999);
        assert!(any_darker, "AO should darken at least one vertex");
    }

    #[test]
    fn generate_mesh_air_only_produces_nothing() {
        let size = Vector3i::new(3, 3, 3);
        let buf = vec![0u16; (size.x * size.y * size.z) as usize];
        let lib = full_cube_library_baked();
        let mut out = vec![BlockyArrays::new()];
        generate_mesh(
            &mut out,
            &buf,
            size,
            &lib,
            false,
            DEFAULT_BAKED_OCCLUSION_DARKNESS,
        );
        assert_eq!(out[0].vertex_count(), 0);
        assert!(out[0].indices.is_empty());
    }

    #[test]
    fn color_mul_is_component_wise() {
        let a = Color::new(0.5, 0.5, 0.5, 1.0);
        let b = Color::new(0.8, 0.8, 0.8, 1.0);
        let c = color_mul(a, b);
        assert!((c.r - 0.4).abs() < 1e-5);
        assert!((c.g - 0.4).abs() < 1e-5);
        assert!((c.b - 0.4).abs() < 1e-5);
    }
}
