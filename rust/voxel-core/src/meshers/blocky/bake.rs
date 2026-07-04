//! Blocky model "bake" pass: side-culling matrix generation + cutout-side
//! baking.
//!
//! Ported from `meshers/blocky/voxel_blocky_library_base.cpp` (the bake-related
//! functions, lines ~85–815). Given a populated [`BakedLibrary`] (models with
//! pre-baked side geometry), [`bake_library`] rasterizes each model's side
//! geometry into a 16×16 bitmap, deduplicates the resulting patterns, and builds
//! the `side_pattern_culling` occlusion matrix in [`BakedLibrary`]. It also sets
//! each model's `full_sides_mask`, `empty_sides_mask`, `side_pattern_indices`,
//! and `contributes_to_ao`.
//!
//! This is a parity port: the goal is identical culling/occlusion decisions to
//! the C++ implementation. The C++ source uses `RASTER_SIZE = 32`
//! (`std::bitset<1024>`); per the port spec we use `RASTER_SIZE = 16`
//! (`[u64; 4]` = 256 bits). Output is identical for any model whose side
//! geometry fully covers (or fully misses) a face — i.e. the common cube case.
//! Only sub-quad (cutout) shapes can rasterize slightly differently, and only
//! in edge precision.
//
// Index-based `for` loops and element-wise slice copies mirror the C++ source
// 1:1 to make the parity port easy to diff. Clippy's iterator/copy_from_slice
// suggestions would obscure that correspondence, so they're silenced here.
#![allow(
    clippy::needless_range_loop,
    clippy::manual_memcpy,
    clippy::collapsible_if
)]

use crate::constants::cube_tables::{Side, OPPOSITE_SIDE};
use crate::math::triangle::{get_triangle_barycentric_coordinates, is_point_in_triangle};
use crate::math::{funcs, vector2::math as v2math, Box2f, Vector2f, Vector3f};
use crate::meshers::blocky::baked_library::{self, BakedLibrary, BakedModel, BakedModelMesh};
use crate::meshers::blocky::mesher::{is_face_visible, is_face_visible_regardless_of_shape};
use crate::meshers::blocky::{SideSurface, MAX_SURFACES};

/// Resolution of the per-side raster bitmap. The C++ source uses 32; this port
/// uses 16 per the port instructions. `RASTER_SIZE²` must equal `BITMAP_BITS`.
/// Output is identical to C++ for any model whose side geometry fully covers
/// (or fully misses) a face — i.e. the common cube case.
pub const RASTER_SIZE: usize = 16;

/// Number of bits in a side raster bitmap (`RASTER_SIZE * RASTER_SIZE`).
pub const BITMAP_BITS: usize = RASTER_SIZE * RASTER_SIZE;

/// A fixed-size `BITMAP_BITS`-bit bitmap (stand-in for `std::bitset<256>`).
/// Stored as four `u64` words. Bit `i = x + y * RASTER_SIZE` is in word `i/64`
/// at bit `i%64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideBitmap([u64; BITMAP_BITS.div_ceil(64)]);

// Compile-time guarantee that 256 bits fit in the chosen word count.
const _: () = assert!(BITMAP_BITS.div_ceil(64) == 4);

impl SideBitmap {
    /// All bits clear.
    pub const fn zero() -> Self {
        Self([0; 4])
    }

    /// All bits set.
    pub const fn full() -> Self {
        Self([u64::MAX; 4])
    }

    /// Set bit `i` to 1.
    #[inline]
    pub fn set(&mut self, i: usize) {
        debug_assert!(i < BITMAP_BITS);
        self.0[i / 64] |= 1 << (i % 64);
    }

    /// Get bit `i`.
    #[inline]
    pub fn get(&self, i: usize) -> bool {
        debug_assert!(i < BITMAP_BITS);
        (self.0[i / 64] >> (i % 64)) & 1 != 0
    }

    /// True if any bit is set.
    #[inline]
    pub fn any(&self) -> bool {
        self.0.iter().any(|&w| w != 0)
    }

    /// True if all bits are set.
    #[inline]
    pub fn all(&self) -> bool {
        self.0.iter().all(|&w| w == u64::MAX)
    }

    /// Bitwise AND.
    #[inline]
    pub fn and(self, other: Self) -> Self {
        Self([
            self.0[0] & other.0[0],
            self.0[1] & other.0[1],
            self.0[2] & other.0[2],
            self.0[3] & other.0[3],
        ])
    }
}

// ---------------------------------------------------------------------------
// Geometry helpers (anonymous-namespace functions in C++)
// ---------------------------------------------------------------------------

/// `3---2`
/// `|   |`
/// `0---1`
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)] // i1/i3 are part of the C++ struct's wire shape (parity).
struct QuadIndices {
    i0: u32,
    i1: u32,
    i2: u32,
    i3: u32,
}

impl QuadIndices {
    fn to_box(self, vertices: &[Vector2f]) -> Box2f {
        let p0 = vertices[self.i0 as usize];
        let p2 = vertices[self.i2 as usize];
        Box2f::from_min_max(p0, p2)
    }
}

/// `detect_single_quad` — returns `true` and fills `out` if `vertices`/`indices`
/// describe a single axis-aligned quad (4 vertices, 6 indices = 2 triangles).
fn detect_single_quad(vertices: &[Vector2f], indices: &[i32], out: &mut QuadIndices) -> bool {
    if vertices.len() != 4 {
        return false;
    }
    if indices.len() != 6 {
        return false;
    }
    let mut minp = vertices[0];
    let mut maxp = minp;
    for &v in vertices {
        minp = v2math::min(v, minp);
        maxp = v2math::max(v, maxp);
    }
    let p0 = minp;
    let p1 = Vector2f::new(maxp.x, minp.y);
    let p2 = maxp;
    let p3 = Vector2f::new(minp.x, maxp.y);

    let mut i0: i32 = -1;
    let mut i1: i32 = -1;
    let mut i2: i32 = -1;
    let mut i3: i32 = -1;

    for (i, &v) in vertices.iter().enumerate() {
        if funcs::is_equal_approx(v.x, p0.x) && funcs::is_equal_approx(v.y, p0.y) {
            i0 = i as i32;
        } else if funcs::is_equal_approx(v.x, p1.x) && funcs::is_equal_approx(v.y, p1.y) {
            i1 = i as i32;
        } else if funcs::is_equal_approx(v.x, p2.x) && funcs::is_equal_approx(v.y, p2.y) {
            i2 = i as i32;
        } else if funcs::is_equal_approx(v.x, p3.x) && funcs::is_equal_approx(v.y, p3.y) {
            i3 = i as i32;
        } else {
            return false;
        }
    }

    if i0 == -1 || i1 == -1 || i2 == -1 || i3 == -1 {
        return false;
    }

    *out = QuadIndices {
        i0: i0 as u32,
        i1: i1 as u32,
        i2: i2 as u32,
        i3: i3 as u32,
    };
    true
}

/// `flip_winding` — reverse each triangle's winding by swapping indices 1↔2.
fn flip_winding(indices: &mut [i32]) {
    debug_assert!(indices.len().is_multiple_of(3));
    let mut i = 0;
    while i < indices.len() {
        indices.swap(i + 1, i + 2);
        i += 3;
    }
}

/// `to_2d` (axis-pair overload) — project `src` 3D positions onto 2D using the
/// given source axes.
fn to_2d_axes(src: &[Vector3f], dst: &mut [Vector2f], src_x_axis: usize, src_y_axis: usize) {
    debug_assert!(src.len() == dst.len());
    for (i, &srcv) in src.iter().enumerate() {
        dst[i] = Vector2f::new(srcv[src_x_axis], srcv[src_y_axis]);
    }
}

/// `to_2d` (side overload) — project side `side`'s 3D positions to 2D, picking
/// the two in-plane axes.
fn to_2d_side(src: &[Vector3f], dst: &mut [Vector2f], side: u8) {
    let (x_axis, y_axis) = side_to_2d_axes(side);
    to_2d_axes(src, dst, x_axis, y_axis);
}

/// Returns the `(x_axis, y_axis)` pair used to project side `side` into 2D.
/// Matches the `switch (side)` in C++ `to_2d`.
fn side_to_2d_axes(side: u8) -> (usize, usize) {
    // Axis indices: 0=X, 1=Y, 2=Z.
    match side {
        x if x == Side::Left as u8 || x == Side::Right as u8 => (2, 1), // Z, Y
        y if y == Side::Bottom as u8 || y == Side::Top as u8 => (0, 2), // X, Z
        z if z == Side::Back as u8 || z == Side::Front as u8 => (0, 1), // X, Y
        _ => (0, 1),
    }
}

/// `to_3d` (axis overload).
#[allow(clippy::too_many_arguments)]
fn to_3d_axes(
    src: &[Vector2f],
    dst: &mut [Vector3f],
    dst_x_axis: usize,
    dst_y_axis: usize,
    dst_z_axis: usize,
    z: f32,
) {
    debug_assert!(src.len() == dst.len());
    for (i, &srcv) in src.iter().enumerate() {
        let mut dstv = Vector3f::new(0.0, 0.0, 0.0);
        dstv[dst_x_axis] = srcv.x;
        dstv[dst_y_axis] = srcv.y;
        dstv[dst_z_axis] = z;
        dst[i] = dstv;
    }
}

/// `to_3d` (side overload) — embed 2D coordinates back into the side's plane.
fn to_3d_side(src: &[Vector2f], dst: &mut [Vector3f], side: u8) {
    match side {
        x if x == Side::Left as u8 => to_3d_axes(src, dst, 2, 1, 0, 0.0),
        x if x == Side::Right as u8 => to_3d_axes(src, dst, 2, 1, 0, 1.0),
        y if y == Side::Bottom as u8 => to_3d_axes(src, dst, 0, 2, 1, 0.0),
        y if y == Side::Top as u8 => to_3d_axes(src, dst, 0, 2, 1, 1.0),
        z if z == Side::Back as u8 => to_3d_axes(src, dst, 0, 1, 2, 0.0),
        z if z == Side::Front as u8 => to_3d_axes(src, dst, 0, 1, 2, 1.0),
        _ => to_3d_axes(src, dst, 0, 1, 2, 0.0),
    }
}

/// `get_side_geometry_2d_all_surfaces` — concatenate all surfaces' side geometry
/// for `side` into a single 2D vertex/index list (indices rebased by vertex
/// offset). Used by the cutout pass.
fn get_side_geometry_2d_all_surfaces(
    model: &BakedModelMesh,
    side: u8,
    out_vertices: &mut Vec<Vector2f>,
    out_indices: &mut Vec<i32>,
) {
    let side_surfaces = &model.sides_surfaces[side as usize];

    let mut vertex_count = 0usize;
    let mut index_count = 0usize;
    for surface_index in 0..model.surface_count as usize {
        let ss = &side_surfaces[surface_index];
        vertex_count += ss.positions.len();
        index_count += ss.indices.len();
    }

    out_vertices.clear();
    out_vertices.resize(vertex_count, Vector2f::new(0.0, 0.0));
    out_indices.clear();
    out_indices.resize(index_count, 0);

    let mut vertex_start = 0usize;
    let mut index_start = 0usize;
    for surface_index in 0..model.surface_count as usize {
        let ss = &side_surfaces[surface_index];
        let nverts = ss.positions.len();
        let nidx = ss.indices.len();

        let dst_v = &mut out_vertices[vertex_start..vertex_start + nverts];
        to_2d_side(&ss.positions, dst_v, side);

        for (k, &idx) in ss.indices.iter().enumerate() {
            out_indices[index_start + k] = idx + vertex_start as i32;
        }

        vertex_start += nverts;
        index_start += nidx;
    }
}

/// `quads_to_triangles` — append two triangles (6 verts) per quad.
fn quads_to_triangles(quads: &[Box2f], vertices: &mut Vec<Vector2f>) {
    vertices.reserve(quads.len() * 6);
    for &quad in quads {
        // 3---2
        // |   |
        // 0---1
        let p0 = quad.min;
        let p2 = quad.max;
        let p1 = Vector2f::new(p2.x, p0.y);
        let p3 = Vector2f::new(p0.x, p2.y);
        vertices.push(p0);
        vertices.push(p1);
        vertices.push(p2);
        vertices.push(p0);
        vertices.push(p2);
        vertices.push(p3);
    }
}

/// `find_approx` — index of the first vertex approximately equal to `p_v`, or
/// `None`.
fn find_approx(vertices: &[Vector2f], p_v: Vector2f) -> Option<usize> {
    for (i, &v) in vertices.iter().enumerate() {
        if funcs::is_equal_approx(v.x, p_v.x) && funcs::is_equal_approx(v.y, p_v.y) {
            return Some(i);
        }
    }
    None
}

/// `index_triangles` — weld near-duplicate vertices, emitting indexed geometry.
fn index_triangles(
    src_vertices: &[Vector2f],
    dst_vertices: &mut Vec<Vector2f>,
    dst_indices: &mut Vec<i32>,
) {
    for &srcv in src_vertices {
        let dsti = match find_approx(dst_vertices, srcv) {
            Some(i) => i,
            None => {
                dst_vertices.push(srcv);
                dst_vertices.len() - 1
            }
        };
        dst_indices.push(dsti as i32);
    }
}

/// `grow_triangle` — expand a triangle outward from its centroid by factor
/// `by` (used to fight float precision in point-in-triangle tests).
fn grow_triangle(a: &mut Vector2f, b: &mut Vector2f, c: &mut Vector2f, by: f32) {
    let m = (1.0 / 3.0) * (*a + *b + *c);
    *a += by * (*a - m);
    *b += by * (*b - m);
    *c += by * (*c - m);
}

/// `find_triangle` — find the triangle (in `indices`) containing `pos`. Returns
/// its three vertex indices.
fn find_triangle(vertices: &[Vector2f], indices: &[i32], pos: Vector2f) -> Option<[usize; 3]> {
    debug_assert!(indices.len().is_multiple_of(3));
    let mut ii = 0;
    while ii < indices.len() {
        let i0 = indices[ii] as usize;
        let i1 = indices[ii + 1] as usize;
        let i2 = indices[ii + 2] as usize;

        let mut p0 = vertices[i0];
        let mut p1 = vertices[i1];
        let mut p2 = vertices[i2];

        grow_triangle(&mut p0, &mut p1, &mut p2, 0.0001);

        if is_point_in_triangle(pos, p0, p1, p2) {
            return Some([i0, i1, i2]);
        }
        ii += 3;
    }
    None
}

/// `interpolate_attributes_assume_no_seams` — recover UVs (and tangents, if
/// present) at `interp_vertices` by barycentric interpolation over the source
/// mesh.
fn interpolate_attributes_assume_no_seams(
    src_vertices: &[Vector2f],
    src_indices: &[i32],
    src_uvs: &[Vector2f],
    src_tangents: &[f32],
    interp_vertices: &[Vector2f],
    interp_uvs: &mut [Vector2f],
    interp_tangents: &mut [f32],
) {
    debug_assert!(src_indices.len().is_multiple_of(3));
    let has_tangents = !src_tangents.is_empty();

    for (i, &interp_pos) in interp_vertices.iter().enumerate() {
        let tri = match find_triangle(src_vertices, src_indices, interp_pos) {
            Some(t) => t,
            None => {
                // C++ logs an error and zeros the UV.
                interp_uvs[i] = Vector2f::new(0.0, 0.0);
                continue;
            }
        };
        let p0 = src_vertices[tri[0]];
        let p1 = src_vertices[tri[1]];
        let p2 = src_vertices[tri[2]];
        let weights = get_triangle_barycentric_coordinates(p0, p1, p2, interp_pos);

        let uv0 = src_uvs[tri[0]];
        let uv1 = src_uvs[tri[1]];
        let uv2 = src_uvs[tri[2]];
        interp_uvs[i] = Vector2f::new(
            uv0.x * weights.x + uv1.x * weights.y + uv2.x * weights.z,
            uv0.y * weights.x + uv1.y * weights.y + uv2.y * weights.z,
        );

        if has_tangents {
            for c in 0..4usize {
                let t0 = src_tangents[tri[0] * 4 + c];
                let t1 = src_tangents[tri[1] * 4 + c];
                let t2 = src_tangents[tri[2] * 4 + c];
                interp_tangents[i * 4 + c] = t0 * weights.x + t1 * weights.y + t2 * weights.z;
            }
        }
    }
}

/// `blocky::generate_cutout_side_surface` — subtract `other_quad` from
/// `side_surface`'s quad, producing the cutout geometry in `cut_side_surface`.
/// Only handles the single-quad case (returns an empty result otherwise).
pub(crate) fn generate_cutout_side_surface(
    side_surface: &SideSurface,
    side: u8,
    other_quad: Box2f,
    cut_side_surface: &mut SideSurface,
) {
    let mut vertices_2d = vec![Vector2f::new(0.0, 0.0); side_surface.positions.len()];
    to_2d_side(&side_surface.positions, &mut vertices_2d, side);

    let mut quad_indices = QuadIndices::default();
    let is_quad = detect_single_quad(&vertices_2d, &side_surface.indices, &mut quad_indices);
    if !is_quad {
        return;
    }
    let quad = quad_indices.to_box(&vertices_2d);

    let quads = quad.difference(other_quad);

    let mut cut_vertices_2d_non_indexed = Vec::new();
    quads_to_triangles(&quads, &mut cut_vertices_2d_non_indexed);

    let mut cut_vertices_2d: Vec<Vector2f> = Vec::new();
    let mut cut_indices: Vec<i32> = Vec::new();
    index_triangles(
        &cut_vertices_2d_non_indexed,
        &mut cut_vertices_2d,
        &mut cut_indices,
    );

    let mut cut_uvs = vec![Vector2f::new(0.0, 0.0); cut_vertices_2d.len()];
    let has_tangents = !side_surface.tangents.is_empty();
    let mut cut_tangents = if has_tangents {
        vec![0.0f32; cut_vertices_2d.len() * 4]
    } else {
        Vec::new()
    };

    interpolate_attributes_assume_no_seams(
        &vertices_2d,
        &side_surface.indices,
        &side_surface.uvs,
        &side_surface.tangents,
        &cut_vertices_2d,
        &mut cut_uvs,
        &mut cut_tangents,
    );

    if side == Side::Left as u8 || side == Side::Bottom as u8 || side == Side::Front as u8 {
        flip_winding(&mut cut_indices);
    }

    let mut cut_vertices = vec![Vector3f::new(0.0, 0.0, 0.0); cut_vertices_2d.len()];
    to_3d_side(&cut_vertices_2d, &mut cut_vertices, side);

    cut_side_surface.positions = cut_vertices;
    cut_side_surface.indices = cut_indices;
    cut_side_surface.uvs = cut_uvs;
    cut_side_surface.tangents = cut_tangents;
}

/// `blocky::generate_model_cutout_sides` — for each neighbor model that
/// partially occludes this model's sides, pre-compute cutout side surfaces.
pub(crate) fn generate_model_cutout_sides(
    model_data: &mut BakedModel,
    model_id: u16,
    lib: &BakedLibrary,
) {
    let model_count = lib.models.len();
    for other_model_id in 0..model_count {
        let other_model_id = other_model_id as u16;
        if other_model_id == model_id {
            continue;
        }
        let other_model_data = &lib.models[other_model_id as usize];
        if !other_model_data.culls_neighbors {
            continue;
        }

        for side in 0..Side::COUNT as u8 {
            // Test if the face is totally occluded first.
            if !is_face_visible(lib, model_data, other_model_id as u32, side as i32) {
                continue;
            }
            if is_face_visible_regardless_of_shape(model_data, other_model_data) {
                continue;
            }

            // The face is partially or totally visible.
            let other_side = OPPOSITE_SIDE[side as usize];

            let mut other_all_vertices_2d: Vec<Vector2f> = Vec::new();
            let mut other_all_indices: Vec<i32> = Vec::new();
            get_side_geometry_2d_all_surfaces(
                &other_model_data.model,
                other_side,
                &mut other_all_vertices_2d,
                &mut other_all_indices,
            );

            let mut other_quad_indices = QuadIndices::default();
            let other_is_quad = detect_single_quad(
                &other_all_vertices_2d,
                &other_all_indices,
                &mut other_quad_indices,
            );
            if !other_is_quad {
                continue;
            }
            let other_quad = other_quad_indices.to_box(&other_all_vertices_2d);

            let other_side_shape_id =
                other_model_data.model.side_pattern_indices[other_side as usize];

            // Build cutout surfaces locally, then move into the model's map if
            // non-empty.
            let mut cut_surfaces: [SideSurface; MAX_SURFACES] = Default::default();

            let side_surfaces = &model_data.model.sides_surfaces[side as usize];
            for surface_index in 0..model_data.model.surface_count as usize {
                let ss = &side_surfaces[surface_index];
                generate_cutout_side_surface(
                    ss,
                    side,
                    other_quad,
                    &mut cut_surfaces[surface_index],
                );
            }

            if cut_surfaces.iter().any(|s| !s.indices.is_empty()) {
                // Record the cutout. We store it keyed by neighbor shape id.
                model_data
                    .cutout_side_surfaces
                    .entry((side, other_side_shape_id))
                    .or_default()
                    .extend(
                        cut_surfaces
                            .iter()
                            .filter(|&s| !s.indices.is_empty())
                            .cloned(),
                    );
            }
        }
    }
}

/// `blocky::generate_library_cutout_sides` — drive cutout generation for every
/// model that opted into it.
fn generate_library_cutout_sides(lib: &mut BakedLibrary) {
    let model_count = lib.models.len();
    for model_id in 0..model_count {
        // borrow-split: take a snapshot of the flag, then conditionally mutate.
        let enabled = lib.models[model_id].cutout_sides_enabled;
        if !enabled {
            continue;
        }
        // Clone the library reference immutably for the read-only neighbor
        // scans; only `models[model_id]` is mutated.
        let model_id_u16 = model_id as u16;
        let lib_ref: *const BakedLibrary = lib;
        // SAFETY: we only mutate `lib.models[model_id]`, and only read other
        // entries. No aliasing of the same element occurs.
        unsafe {
            let lib_imm: &BakedLibrary = &*lib_ref;
            generate_model_cutout_sides(&mut lib.models[model_id], model_id_u16, lib_imm);
        }
    }
}

/// `rasterize_triangle_barycentric` — call `output_func(x, y)` for every grid
/// cell (sampled at its center) covered by triangle `a,b,c`. Coordinates are in
/// raster space (already scaled by `RASTER_SIZE`).
fn rasterize_triangle_barycentric<F: FnMut(usize, usize)>(
    mut a: Vector2f,
    mut b: Vector2f,
    mut c: Vector2f,
    mut output_func: F,
) {
    // Grow the triangle a tiny bit, to help against floating point error.
    grow_triangle(&mut a, &mut b, &mut c, 0.001);

    let min_x = funcs::min3(a.x, b.x, c.x).floor() as i32;
    let min_y = funcs::min3(a.y, b.y, c.y).floor() as i32;
    let max_x = funcs::max3(a.x, b.x, c.x).ceil() as i32;
    let max_y = funcs::max3(a.y, b.y, c.y).ceil() as i32;

    // We test against points centered on grid cells.
    let offset = Vector2f::new(0.5, 0.5);

    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = Vector2f::new(x as f32, y as f32) + offset;
            if is_point_in_triangle(p, a, b, c) {
                output_func(x as usize, y as usize);
            }
        }
    }
}

/// `rasterize_side` — rasterize one side's triangle list into `bitmap`.
fn rasterize_side(vertices: &[Vector3f], indices: &[i32], side: u8, bitmap: &mut SideBitmap) {
    debug_assert!(indices.len().is_multiple_of(3));
    let mut j = 0;
    while j < indices.len() {
        let va = vertices[indices[j] as usize];
        let vb = vertices[indices[j + 1] as usize];
        let vc = vertices[indices[j + 2] as usize];

        // Convert 3D vertices into 2D (same axis selection as the C++ switch).
        let (a, b, c) = match side {
            x if x == Side::Left as u8 || x == Side::Right as u8 => (
                Vector2f::new(va.y, va.z),
                Vector2f::new(vb.y, vb.z),
                Vector2f::new(vc.y, vc.z),
            ),
            y if y == Side::Bottom as u8 || y == Side::Top as u8 => (
                Vector2f::new(va.x, va.z),
                Vector2f::new(vb.x, vb.z),
                Vector2f::new(vc.x, vc.z),
            ),
            z if z == Side::Back as u8 || z == Side::Front as u8 => (
                Vector2f::new(va.x, va.y),
                Vector2f::new(vb.x, vb.y),
                Vector2f::new(vc.x, vc.y),
            ),
            _ => unreachable!("invalid side"),
        };

        let a = a * (RASTER_SIZE as f32);
        let b = b * (RASTER_SIZE as f32);
        let c = c * (RASTER_SIZE as f32);

        let bm: &mut SideBitmap = bitmap;
        rasterize_triangle_barycentric(a, b, c, |x, y| {
            if x < RASTER_SIZE && y < RASTER_SIZE {
                let i = x + y * RASTER_SIZE;
                bm.set(i);
            }
        });
        j += 3;
    }
}

/// `blocky::rasterize_side_all_surfaces` — rasterize every surface's side
/// geometry into `bitmap`.
fn rasterize_side_all_surfaces(model_data: &BakedModel, side_index: u8, bitmap: &mut SideBitmap) {
    let side_surfaces = &model_data.model.sides_surfaces[side_index as usize];
    for surface_index in 0..model_data.model.surface_count as usize {
        let side = &side_surfaces[surface_index];
        rasterize_side(&side.positions, &side.indices, side_index, bitmap);
    }
}

/// Sentinel "no index" value, matching `VoxelBlockyLibraryBase::NULL_INDEX`.
const NULL_INDEX: u32 = u32::MAX;

/// `blocky::generate_side_culling_matrix` — THE KEY FUNCTION. Rasterizes each
/// model's side geometry into a bitmap, deduplicates patterns, builds the
/// occlusion matrix in `BakedLibrary::side_pattern_culling`, and sets each
/// model's `full_sides_mask` / `empty_sides_mask` / `side_pattern_indices` /
/// `contributes_to_ao`.
pub fn generate_side_culling_matrix(baked_data: &mut BakedLibrary) {
    // When two blocky voxels are next to each other, they share a side.
    // Geometry of either side can be culled away if covered by the other, but
    // it's very expensive to do a full polygon check when we build the mesh.
    // So instead, we compute which sides occlude which for every voxel type,
    // and generate culling masks ahead of time, using an approximation.

    #[derive(Clone)]
    struct Pattern {
        bitmap: SideBitmap,
    }

    let mut patterns: Vec<Pattern> = Vec::new();
    let mut full_side_pattern_index: u32 = NULL_INDEX;

    // Gather patterns for each model.
    for model_data in baked_data.models.iter_mut() {
        model_data.contributes_to_ao = true;
        model_data.model.full_sides_mask = 0;
        model_data.model.empty_sides_mask = 0;

        for side in 0..Side::COUNT as u8 {
            let mut bitmap = SideBitmap::zero();

            if model_data.fluid_index != baked_library::NULL_FLUID_INDEX {
                // Fluids don't have per-model static geometry, but their culling
                // rules are still similar to a cube. Fill all bits so the bottom
                // of stacked water voxels still gets culled.
                bitmap = SideBitmap::full();
                // Fluids don't contribute to AO.
                model_data.contributes_to_ao = false;
            } else {
                rasterize_side_all_surfaces(model_data, side, &mut bitmap);
            }

            // Track empty sides.
            if !bitmap.any() {
                model_data.model.empty_sides_mask |= 1 << side;
            }

            // Detect full sides.
            {
                let full_bitmap = SideBitmap::full();
                if bitmap.and(full_bitmap) == full_bitmap && bitmap.all() {
                    model_data.model.full_sides_mask |= 1 << side;
                }
            }

            // Find if the same pattern already exists.
            let mut pattern_index: u32 = NULL_INDEX;
            for (i, p) in patterns.iter().enumerate() {
                if p.bitmap == bitmap {
                    pattern_index = i as u32;
                    break;
                }
            }

            // Get or create pattern.
            if pattern_index == NULL_INDEX {
                pattern_index = patterns.len() as u32;
                patterns.push(Pattern { bitmap });
            }

            if full_side_pattern_index == NULL_INDEX && bitmap.all() {
                full_side_pattern_index = pattern_index;
            }
            if pattern_index != full_side_pattern_index {
                // Non-cube voxels don't contribute to AO at the moment.
                model_data.contributes_to_ao = false;
            }

            model_data.model.side_pattern_indices[side as usize] = pattern_index;
        }
    }

    // Find which pattern occludes which.
    baked_data.side_pattern_count = patterns.len() as u32;
    let n = patterns.len() as u32;
    baked_data.side_pattern_culling.resize((n * n) as usize);
    // `resize` already zero-inits new bits.

    for ai in 0..patterns.len() {
        let pattern_a = &patterns[ai];

        if pattern_a.bitmap.any() {
            // Pattern always occludes itself.
            let i = (ai as u32 + (ai as u32) * n) as usize;
            baked_data.side_pattern_culling.set(i, true);
        }

        for bi in (ai + 1)..patterns.len() {
            let pattern_b = &patterns[bi];
            let res = pattern_a.bitmap.and(pattern_b.bitmap);

            if !res.any() {
                continue;
            }

            let b_occludes_a = res == pattern_a.bitmap;
            let a_occludes_b = res == pattern_b.bitmap;

            // Same patterns? That can't be, they must be unique.
            debug_assert!(!(b_occludes_a && a_occludes_b));

            if a_occludes_b {
                let i = (ai as u32 + (bi as u32) * n) as usize;
                baked_data.side_pattern_culling.set(i, true);
            } else if b_occludes_a {
                let i = (bi as u32 + (ai as u32) * n) as usize;
                baked_data.side_pattern_culling.set(i, true);
            }
        }
    }

    generate_library_cutout_sides(baked_data);
}

/// Public entry point: bake the whole library (side-culling matrix + cutouts).
/// Equivalent to the bake-related portion of `VoxelBlockyLibraryBase::bake()`.
pub fn bake_library(lib: &mut BakedLibrary) {
    generate_side_culling_matrix(lib);
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::cube_tables::{CORNER_POSITION, SIDE_CORNERS, SIDE_QUAD_TRIANGLES};
    use crate::math::{Vector2f, Vector3f};
    use crate::meshers::blocky::baked_library::{BakedLibrary, BakedModel, SideSurface};

    /// Build a full unit-cube side surface for `side`: 4 corners + 2 triangles,
    /// winding per `SIDE_QUAD_TRIANGLES`.
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

    /// A library with index 0 = air (empty) and index 1 = a full opaque cube.
    fn full_cube_library() -> BakedLibrary {
        let air = BakedModel::default(); // empty == true
        let mut cube = BakedModel {
            empty: false,
            culls_neighbors: true,
            contributes_to_ao: true,
            ..Default::default()
        };
        cube.model.surface_count = 1;
        cube.model.surfaces[0].collision_enabled = true;
        for side in 0..Side::COUNT {
            cube.model.sides_surfaces[side][0] = full_cube_side_surface(side);
        }

        BakedLibrary {
            models: vec![air, cube],
            ..Default::default()
        }
    }

    #[test]
    fn side_bitmap_set_get_any_all() {
        let mut bm = SideBitmap::zero();
        assert!(!bm.any());
        assert!(!bm.all());
        bm.set(0);
        bm.set(255);
        assert!(bm.any());
        assert!(bm.get(0));
        assert!(bm.get(255));
        assert!(!bm.get(1));
        assert!(!bm.all());

        let full = SideBitmap::full();
        assert!(full.all());
        assert!(full.any());
    }

    #[test]
    fn side_bitmap_and_equals_identity() {
        let mut a = SideBitmap::zero();
        a.set(5);
        a.set(100);
        let a_and_a = a.and(a);
        assert_eq!(a_and_a, a);
        let empty = SideBitmap::zero();
        assert_eq!(a.and(empty), empty);
    }

    #[test]
    fn detect_single_quad_axis_aligned() {
        let verts = vec![
            Vector2f::new(0.0, 0.0),
            Vector2f::new(1.0, 0.0),
            Vector2f::new(1.0, 1.0),
            Vector2f::new(0.0, 1.0),
        ];
        let idx = vec![0, 1, 2, 0, 2, 3];
        let mut q = QuadIndices::default();
        assert!(detect_single_quad(&verts, &idx, &mut q));
        let box2d = q.to_box(&verts);
        assert_eq!(box2d.min, Vector2f::new(0.0, 0.0));
        assert_eq!(box2d.max, Vector2f::new(1.0, 1.0));
    }

    #[test]
    fn detect_single_quad_rejects_non_quad() {
        let verts = vec![
            Vector2f::new(0.0, 0.0),
            Vector2f::new(0.5, 0.0),
            Vector2f::new(1.0, 1.0),
            Vector2f::new(0.0, 1.0),
        ];
        let idx = vec![0, 1, 2, 0, 2, 3];
        let mut q = QuadIndices::default();
        // The (0.5,0) point is not a corner of the AABB → not a single quad.
        assert!(!detect_single_quad(&verts, &idx, &mut q));
    }

    #[test]
    fn flip_winding_reverses_triangles() {
        let mut idx = vec![0, 1, 2, 3, 4, 5];
        flip_winding(&mut idx);
        assert_eq!(idx, vec![0, 2, 1, 3, 5, 4]);
    }

    #[test]
    fn rasterize_full_cube_side_fills_bitmap() {
        // Rasterize a single full face (the +Z Front side: quad in x,y plane).
        let surface = full_cube_side_surface(Side::Front as usize);
        let mut bm = SideBitmap::zero();
        rasterize_side(
            &surface.positions,
            &surface.indices,
            Side::Front as u8,
            &mut bm,
        );
        assert!(bm.all(), "a full cube side must rasterize to all bits set");
    }

    #[test]
    fn rasterize_empty_side_leaves_bitmap_empty() {
        let surface = SideSurface::default();
        let mut bm = SideBitmap::zero();
        rasterize_side(
            &surface.positions,
            &surface.indices,
            Side::Front as u8,
            &mut bm,
        );
        assert!(!bm.any());
    }

    #[test]
    fn bake_library_full_cube_sets_masks_and_patterns() {
        let mut lib = full_cube_library();
        bake_library(&mut lib);

        let cube = &lib.models[1];
        // All six faces are full quads.
        assert_eq!(cube.model.full_sides_mask, 0b111111);
        // No empty sides.
        assert_eq!(cube.model.empty_sides_mask, 0);
        // A full cube contributes to AO.
        assert!(cube.contributes_to_ao);

        // All sides of the cube share one pattern (the "full" pattern).
        let p0 = cube.model.side_pattern_indices[0];
        for side in 0..Side::COUNT {
            assert_eq!(
                cube.model.side_pattern_indices[side], p0,
                "all cube sides must share the full pattern"
            );
        }

        // The air model has empty sides and an empty pattern distinct from the
        // cube's full pattern.
        let air = &lib.models[0];
        assert_eq!(air.model.empty_sides_mask, 0b111111);
        assert_eq!(air.model.full_sides_mask, 0);
        // Air's pattern is the empty pattern, which is NOT the full pattern, so
        // the bake sets contributes_to_ao = false (non-cube voxels don't
        // contribute to AO). Matches the C++ `generate_side_culling_matrix`.
        assert!(!air.contributes_to_ao);
    }

    #[test]
    fn bake_library_full_cube_occlusion_matrix() {
        let mut lib = full_cube_library();
        bake_library(&mut lib);

        // After baking there are (at least) two patterns: "empty" and "full".
        // The full pattern occludes itself; nothing occludes across empty/full
        // because the AND of empty with anything is empty.
        let cube = &lib.models[1];
        let full_pattern = cube.model.side_pattern_indices[0];
        let air = &lib.models[0];
        let empty_pattern = air.model.side_pattern_indices[0];

        assert_ne!(full_pattern, empty_pattern);
        // Full occludes full.
        assert!(lib.get_side_pattern_occlusion(full_pattern, full_pattern));
        // Empty does not occlude empty (its bitmap has no set bits).
        assert!(!lib.get_side_pattern_occlusion(empty_pattern, empty_pattern));
    }

    #[test]
    fn generate_mesh_helper_imports_compile() {
        // Smoke test that the cross-module visibility helpers are reachable
        // from this module's imports (they are used by the cutout pass).
        use crate::meshers::blocky::mesher::{
            is_face_visible, is_face_visible_according_to_shape,
            is_face_visible_regardless_of_shape,
        };
        let lib = full_cube_library();
        let cube = &lib.models[1];
        let air = &lib.models[0];
        // An air neighbor is visible regardless of shape (air.empty == true).
        assert!(is_face_visible_regardless_of_shape(cube, air));
        // Two opaque cubes are NOT visible regardless of shape.
        assert!(!is_face_visible_regardless_of_shape(cube, cube));
        // is_face_visible: cube next to air → visible.
        assert!(is_face_visible(&lib, cube, 0, Side::Front as i32));
        // is_face_visible_according_to_shape: same full pattern → not visible
        // (the patterns are equal, so the "ai != bi" clause is false and the
        // occlusion clause is false).
        assert!(!is_face_visible_according_to_shape(
            &lib,
            cube,
            cube,
            Side::Front as i32
        ));
    }
}
