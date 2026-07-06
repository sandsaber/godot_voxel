//! Builtin [`VoxelMesher`] implementations wrapping the per-algorithm free
//! functions (`meshers::transvoxel`, `meshers::cubes`, `meshers::blocky`).
//!
//! These adapters let the terrain pipeline drive real meshers through the
//! trait object without rewriting the algorithm code — each mesher is a thin
//! shim that pulls voxels out of [`VoxelBuffer`] and forwards them to the
//! existing free function.
//!
//! Only the transvoxel adapter is wired today. Cubes/blocky adapters are
//! sketched as TODOs because their underlying free functions take custom
//! voxel slices / colour callbacks and need a slightly larger porting
//! surface than fits this commit.

use crate::math::Vector3i;
use crate::meshers::transvoxel::{
    build_regular_mesh, BuildRegularMeshParams, Cache, MeshArrays, RegularMesherInput, MAX_PADDING,
    MIN_PADDING,
};
use crate::meshers::{MesherInput, MesherOutput, Surface, SurfaceArrays, VoxelMesher};
use crate::storage::{ChannelId, VoxelBuffer};

/// `RegularMesherInput` adapter over a [`VoxelBuffer`]'s SDF channel.
///
/// The transvoxel algorithm uses the convention "positive SDF = inside
/// solid" (the C++ `sdf_as_float` negates the snorm-stored value). The Rust
/// `VoxelBuffer` stores SDF with the opposite convention ("positive =
/// outside", matching `SDF_FAR_OUTSIDE = +100`), so the adapter negates on
/// read — same as C++ `sdf_as_float`.
struct VoxelBufferTransvoxelInput<'a> {
    buffer: &'a VoxelBuffer,
    sdf_channel: usize,
    size: Vector3i,
}

impl<'a> VoxelBufferTransvoxelInput<'a> {
    fn new(buffer: &'a VoxelBuffer, sdf_channel: usize) -> Self {
        Self {
            buffer,
            sdf_channel,
            size: buffer.size(),
        }
    }
}

impl<'a> RegularMesherInput for VoxelBufferTransvoxelInput<'a> {
    fn len(&self) -> usize {
        (self.size.x as usize) * (self.size.y as usize) * (self.size.z as usize)
    }

    fn block_size(&self) -> Vector3i {
        self.size
    }

    fn sample_f32(&self, data_index: usize) -> f32 {
        // ZXY layout: index = y + sy*(x + sx*z). Y innermost. Matches the
        // C++ VoxelBuffer memory layout documented in transvoxel/regular.rs.
        let sx = self.size.x as usize;
        let sy = self.size.y as usize;
        let z = data_index / (sx * sy);
        let rem = data_index % (sx * sy);
        let x = rem / sy;
        let y = rem % sy;
        // Negate to flip "positive = outside" into "positive = inside".
        -self
            .buffer
            .get_voxel_f(x as i32, y as i32, z as i32, self.sdf_channel)
    }
}

/// Smooth (SDF) terrain mesher wrapping the transvoxel regular-cell path.
///
/// Produces one [`Surface`] per `build` call (single material, index 0).
/// Vertices/normals/indices are stored in a [`MeshArrays`] wrapped by
/// [`SurfaceArrays::Transvoxel`]. Padding is fixed at the transvoxel
/// algorithm's `MIN_PADDING=1` / `MAX_PADDING=2` requirement.
pub struct TransvoxelMesher {
    sdf_channel: usize,
    cache: Cache,
}

impl Default for TransvoxelMesher {
    fn default() -> Self {
        Self::new()
    }
}

impl TransvoxelMesher {
    pub fn new() -> Self {
        Self {
            sdf_channel: ChannelId::Sdf.index(),
            cache: Cache::default(),
        }
    }

    /// Use a channel other than the default SDF channel.
    pub fn with_sdf_channel(mut self, channel: usize) -> Self {
        self.sdf_channel = channel;
        self
    }
}

impl VoxelMesher for TransvoxelMesher {
    fn build(&mut self, output: &mut MesherOutput, input: &MesherInput<'_>) {
        let transvoxel_input = VoxelBufferTransvoxelInput::new(input.voxels, self.sdf_channel);
        let params = BuildRegularMeshParams {
            lod_index: u32::from(input.lod_index),
            edge_clamp_margin: 0.0,
        };
        let mut arrays = MeshArrays::default();
        build_regular_mesh(&transvoxel_input, &params, &mut self.cache, &mut arrays);
        if input.collision_hint && !arrays.indices.is_empty() {
            output.collision_surface.submesh_vertex_end = arrays.vertices.len() as i32;
            output.collision_surface.submesh_index_end = arrays.indices.len() as i32;
        }
        // Even an empty transvoxel run produces a surface (zero triangles);
        // match C++ which always emits the surface and lets the caller drop
        // empty ones.
        output
            .surfaces
            .push(Surface::new(SurfaceArrays::Transvoxel(arrays), 0));
    }

    fn minimum_padding(&self) -> u32 {
        MIN_PADDING as u32
    }

    fn maximum_padding(&self) -> u32 {
        MAX_PADDING as u32
    }

    fn used_channels_mask(&self) -> u32 {
        1u32 << self.sdf_channel
    }

    fn is_generating_collision_surface(&self) -> bool {
        true
    }
}

/// Blocky colored-cube mesher wrapping the existing greedy-cubes free
/// function. Reads the `Color` channel as 32-bit voxel ids, looks up each
/// id in a [`ColorPalette`], and emits two surfaces (opaque + transparent)
/// matching the C++ `VoxelMesherCubes` output.
pub struct CubesMesher {
    type_channel: usize,
    palette: crate::meshers::cubes::palette::ColorPalette,
    /// When `true`, uses the greedy rectangle-merging path. When `false`,
    /// emits one quad per face (the simpler reference path). Greedy is the
    /// C++ default.
    greedy: bool,
}

impl Default for CubesMesher {
    fn default() -> Self {
        Self::new()
    }
}

impl CubesMesher {
    pub fn new() -> Self {
        Self {
            type_channel: ChannelId::Color.index(),
            palette: crate::meshers::cubes::palette::ColorPalette::default(),
            greedy: true,
        }
    }

    pub fn with_palette(mut self, palette: crate::meshers::cubes::palette::ColorPalette) -> Self {
        self.palette = palette;
        self
    }

    pub fn with_greedy(mut self, greedy: bool) -> Self {
        self.greedy = greedy;
        self
    }

    pub fn with_type_channel(mut self, channel: usize) -> Self {
        self.type_channel = channel;
        self
    }

    /// Extract the typed-channel slice the cubes mesher expects. The C++
    /// runtime packs voxels as `u32` regardless of the on-disk depth; we do
    /// the same by reading each voxel via `get_voxel` (returns `u64`).
    fn extract_voxel_slice(buffer: &VoxelBuffer, channel: usize) -> Vec<u32> {
        let size = buffer.size();
        let mut out = Vec::with_capacity((size.x as usize) * (size.y as usize) * (size.z as usize));
        // ZXY order matches the cubes free function's `index = y + sy*(x + sx*z)`.
        for z in 0..size.z {
            for x in 0..size.x {
                for y in 0..size.y {
                    out.push(buffer.get_voxel(x, y, z, channel) as u32);
                }
            }
        }
        out
    }
}

impl VoxelMesher for CubesMesher {
    fn build(&mut self, output: &mut MesherOutput, input: &MesherInput<'_>) {
        use crate::meshers::cubes::greedy::MATERIAL_COUNT;
        let voxels = Self::extract_voxel_slice(input.voxels, self.type_channel);
        let size = input.voxels.size();
        let block_size = [size.x, size.y, size.z];
        let palette = self.palette.clone();
        let color_func = move |raw: u32| palette.get_color8(raw as u8);

        let mut arrays: [crate::meshers::cubes::arrays::CubesArrays; MATERIAL_COUNT] =
            Default::default();
        if self.greedy {
            crate::meshers::cubes::greedy::build_greedy_cubes(
                &mut arrays,
                &voxels,
                block_size,
                color_func,
            );
        } else {
            crate::meshers::cubes::simple::build_simple_cubes(
                &mut arrays,
                &voxels,
                block_size,
                color_func,
            );
        }

        // Two surfaces: opaque (index 0) and transparent (index 1). Both are
        // always emitted to match the C++ `Output::surfaces` shape; the
        // terrain layer can drop empty ones.
        output.surfaces.push(Surface::new(
            SurfaceArrays::Cubes(std::mem::take(&mut arrays[0])),
            0,
        ));
        output.surfaces.push(Surface::new(
            SurfaceArrays::Cubes(std::mem::take(&mut arrays[1])),
            1,
        ));
    }

    fn minimum_padding(&self) -> u32 {
        crate::meshers::cubes::greedy::PADDING as u32
    }

    fn maximum_padding(&self) -> u32 {
        crate::meshers::cubes::greedy::PADDING as u32
    }

    fn used_channels_mask(&self) -> u32 {
        1u32 << self.type_channel
    }

    fn supports_lod(&self) -> bool {
        false
    }
}

/// Voxel-model blocky mesher wrapping the existing `blocky::mesher::generate_mesh`
/// free function. Reads the `Type` channel as 16-bit voxel ids, looks up each
/// id in a [`BakedLibrary`] (voxel model library + side-culling matrix), and
/// emits one surface per material the library uses.
///
/// The library is shared via `Arc` so multiple terrain instances can use the
/// same baked data without re-baking. A library built with
/// [`BakedLibrary::default`] is empty (no models); callers must populate it
/// and run [`blocky::bake_library`] before passing it in.
pub struct BlockyMesher {
    type_channel: usize,
    library: std::sync::Arc<crate::meshers::blocky::baked_library::BakedLibrary>,
    /// 0fps-style corner ambient occlusion toggle (C++ default `true`).
    bake_occlusion: bool,
    /// AO strength (C++ default `0.8`).
    baked_occlusion_darkness: f32,
}

impl BlockyMesher {
    /// Build a mesher around a pre-baked library. The library must already
    /// have run `blocky::bake_library` so its side-culling matrix is valid.
    pub fn new(
        library: std::sync::Arc<crate::meshers::blocky::baked_library::BakedLibrary>,
    ) -> Self {
        Self {
            type_channel: ChannelId::Type.index(),
            library,
            bake_occlusion: true,
            baked_occlusion_darkness: 0.8,
        }
    }

    pub fn with_type_channel(mut self, channel: usize) -> Self {
        self.type_channel = channel;
        self
    }

    pub fn with_occlusion(mut self, enabled: bool, darkness: f32) -> Self {
        self.bake_occlusion = enabled;
        self.baked_occlusion_darkness = darkness;
        self
    }

    /// Extract the typed-channel slice the blocky mesher expects (`&[u16]`).
    /// Voxels are read as `u64` then narrowed, matching the C++ runtime.
    fn extract_voxel_slice(buffer: &VoxelBuffer, channel: usize) -> Vec<u16> {
        let size = buffer.size();
        let mut out = Vec::with_capacity((size.x as usize) * (size.y as usize) * (size.z as usize));
        for z in 0..size.z {
            for x in 0..size.x {
                for y in 0..size.y {
                    out.push(buffer.get_voxel(x, y, z, channel) as u16);
                }
            }
        }
        out
    }
}

impl VoxelMesher for BlockyMesher {
    fn build(&mut self, output: &mut MesherOutput, input: &MesherInput<'_>) {
        use crate::meshers::blocky::mesher::{generate_mesh, generate_mesh_with_collision};
        let voxels = Self::extract_voxel_slice(input.voxels, self.type_channel);
        let size = input.voxels.size();
        let material_count = self.library.indexed_materials_count.max(1) as usize;
        let mut arrays: Vec<crate::meshers::blocky::mesher::BlockyArrays> =
            (0..material_count).map(|_| Default::default()).collect();
        if input.collision_hint {
            let mut collision_arrays = crate::meshers::blocky::mesher::BlockyArrays::default();
            generate_mesh_with_collision(
                &mut arrays,
                &mut collision_arrays,
                &voxels,
                size,
                &self.library,
                self.bake_occlusion,
                self.baked_occlusion_darkness,
            );
            output.collision_surface.positions = collision_arrays.positions;
            output.collision_surface.indices = collision_arrays.indices;
        } else {
            generate_mesh(
                &mut arrays,
                &voxels,
                size,
                &self.library,
                self.bake_occlusion,
                self.baked_occlusion_darkness,
            );
        }
        for (material_index, arrays) in arrays.into_iter().enumerate() {
            output.surfaces.push(Surface::new(
                SurfaceArrays::Blocky(arrays),
                material_index as u16,
            ));
        }
    }

    fn minimum_padding(&self) -> u32 {
        crate::meshers::blocky::mesher::PADDING as u32
    }

    fn maximum_padding(&self) -> u32 {
        crate::meshers::blocky::mesher::PADDING as u32
    }

    fn used_channels_mask(&self) -> u32 {
        1u32 << self.type_channel
    }

    fn supports_lod(&self) -> bool {
        false
    }

    fn is_generating_collision_surface(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockyMesher, CubesMesher, TransvoxelMesher};
    use crate::constants::cube_tables::{Side, CORNER_POSITION, SIDE_CORNERS, SIDE_QUAD_TRIANGLES};
    use crate::math::{Vector2f, Vector3f, Vector3i};
    use crate::meshers::blocky::baked_library::{BakedLibrary, BakedModel};
    use crate::meshers::blocky::SideSurface;
    use crate::meshers::transvoxel::{MAX_PADDING, MIN_PADDING};
    use crate::meshers::{MesherInput, MesherOutput, SurfaceArrays, VoxelMesher};
    use crate::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat};

    /// Build a `VoxelBuffer` of `inner³` voxels (padded with the transvoxel
    /// halo) containing an SDF sphere of `radius`, centred in the inner
    /// region. SDF convention: positive = outside (matches `SDF_FAR_OUTSIDE`).
    fn sphere_buffer(inner: i32, radius: f32) -> VoxelBuffer {
        let padded = inner + MIN_PADDING + MAX_PADDING;
        let mut buf = VoxelBuffer::with_size(Vector3i::splat(padded));
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        format.configure_buffer(&mut buf);

        let centre = (inner as f32) * 0.5;
        for z in 0..padded {
            for x in 0..padded {
                for y in 0..padded {
                    let ix = x as f32 - MIN_PADDING as f32;
                    let iy = y as f32 - MIN_PADDING as f32;
                    let iz = z as f32 - MIN_PADDING as f32;
                    let distance =
                        ((ix - centre).powi(2) + (iy - centre).powi(2) + (iz - centre).powi(2))
                            .sqrt()
                            - radius;
                    buf.set_voxel_f(distance, x, y, z, ChannelId::Sdf.index());
                }
            }
        }
        buf
    }

    #[test]
    fn transvoxel_mesher_produces_substantial_geometry_for_sphere() {
        let mut mesher = TransvoxelMesher::new();
        let voxels = sphere_buffer(16, 6.0);
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        assert_eq!(output.surfaces.len(), 1);
        // The transvoxel sphere test in tests/transvoxel_sphere.rs asserts
        // > 100 vertices for the same configuration. Mirror that floor.
        let total_vertices = output.total_vertex_count();
        assert!(
            total_vertices > 100,
            "expected substantial mesh for an r=6 sphere in 16³ block, got {total_vertices}"
        );
        assert!(output.total_triangle_count() > 0);
    }

    #[test]
    fn transvoxel_mesher_emits_empty_surface_for_uniform_outside_volume() {
        // A buffer filled entirely with SDF_FAR_OUTSIDE (all air) has no
        // surface-crossing cells and produces an empty mesh.
        let mut mesher = TransvoxelMesher::new();
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(16));
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        format.configure_buffer(&mut voxels);
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        assert_eq!(output.surfaces.len(), 1);
        assert_eq!(output.total_triangle_count(), 0);
    }

    #[test]
    fn transvoxel_mesher_padding_matches_algorithm_constants() {
        let mesher = TransvoxelMesher::new();
        assert_eq!(mesher.minimum_padding(), MIN_PADDING as u32);
        assert_eq!(mesher.maximum_padding(), MAX_PADDING as u32);
        assert_eq!(mesher.used_channels_mask(), 1u32 << ChannelId::Sdf.index());
    }

    /// Verify the mesher is `Send + Sync` (required by `VoxelMesher`) so it
    /// can live behind `Arc<Mutex<Box<dyn VoxelMesher>>>` in MeshingDependency.
    #[test]
    fn transvoxel_mesher_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TransvoxelMesher>();
    }

    #[test]
    fn transvoxel_collision_hint_populates_collision_submesh_range() {
        let mut mesher = TransvoxelMesher::new();
        let voxels = sphere_buffer(16, 6.0);
        let mut input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        input.collision_hint = true;

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        assert!(mesher.is_generating_collision_surface());
        assert!(output.total_triangle_count() > 0);
        assert!(output.collision_surface.submesh_vertex_end > 0);
        assert!(output.collision_surface.submesh_index_end > 0);
    }

    /// Vertex positions should land in world space (origin_in_voxels offset
    /// applied). The transvoxel free function uses scaled block-local coords;
    /// the wrapper currently forwards them as-is — so for `lod_index==0` and
    /// a zero origin, positions should still be non-negative within the block.
    #[test]
    fn transvoxel_mesher_vertex_positions_are_within_block_for_zero_origin() {
        let mut mesher = TransvoxelMesher::new();
        let voxels = sphere_buffer(16, 6.0);
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        let arrays = match &output.surfaces[0].arrays {
            SurfaceArrays::Transvoxel(a) => a,
            _ => unreachable!(),
        };
        let padded_extent = (16 + MIN_PADDING + MAX_PADDING) as f32;
        assert!(arrays.vertices.iter().all(|p| {
            let Vector3f { x, y, z } = *p;
            x >= 0.0
                && y >= 0.0
                && z >= 0.0
                && x < padded_extent
                && y < padded_extent
                && z < padded_extent
        }));
    }

    /// Build a small VoxelBuffer filled with a single solid voxel type in the
    /// interior and air (0) on the padding halo — the typical input
    /// `CubesMesher` sees after the gather step.
    fn cubes_input_buffer() -> VoxelBuffer {
        // Padded block: PADDING(1) interior 2³ + PADDING(1) on each side.
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(2 + 2));
        let channel = ChannelId::Color.index();
        // Fill the interior (1..3)³ with voxel id 1.
        for z in 1..3 {
            for x in 1..3 {
                for y in 1..3 {
                    voxels.set_voxel(1, x, y, z, channel);
                }
            }
        }
        voxels
    }

    #[test]
    fn cubes_mesher_produces_two_surfaces_for_a_solid_block() {
        let mut mesher = CubesMesher::new();
        let voxels = cubes_input_buffer();
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        // Two surfaces emitted (opaque material 0, transparent material 1).
        assert_eq!(output.surfaces.len(), 2);
        // The opaque surface should have geometry for a solid 2³ block.
        let opaque_vertices = output.surfaces[0].arrays.vertex_count();
        assert!(
            opaque_vertices > 0,
            "expected opaque geometry for a solid block, got {opaque_vertices}"
        );
        // Material indices are 0 and 1 in order.
        assert_eq!(output.surfaces[0].material_index, 0);
        assert_eq!(output.surfaces[1].material_index, 1);
    }

    #[test]
    fn cubes_mesher_emits_empty_surfaces_for_air_block() {
        let mut mesher = CubesMesher::new();
        // All-zero Type channel → no solid voxels → no faces.
        let voxels = VoxelBuffer::with_size(Vector3i::splat(4));
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        assert_eq!(output.total_triangle_count(), 0);
    }

    #[test]
    fn cubes_mesher_padding_and_channels_match_constants() {
        let mesher = CubesMesher::new();
        assert_eq!(mesher.minimum_padding(), 1);
        assert_eq!(mesher.maximum_padding(), 1);
        assert_eq!(
            mesher.used_channels_mask(),
            1u32 << ChannelId::Color.index()
        );
    }

    #[test]
    fn cubes_mesher_reports_lod_unsupported_until_lod_inputs_are_used() {
        let mesher = CubesMesher::new();

        assert!(!mesher.supports_lod());
    }

    #[test]
    fn cubes_mesher_supports_non_greedy_simple_path() {
        let mut mesher = CubesMesher::new().with_greedy(false);
        let voxels = cubes_input_buffer();
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        // Simple path emits one quad per face — more vertices than greedy,
        // but still non-empty for a solid block.
        assert!(output.total_triangle_count() > 0);
    }

    #[test]
    fn cubes_mesher_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CubesMesher>();
    }

    fn empty_blocky_library() -> std::sync::Arc<crate::meshers::blocky::baked_library::BakedLibrary>
    {
        // Default-constructed library is empty (no models), so the mesher
        // emits no geometry regardless of input. Useful for testing the
        // adapter wiring without pulling in the full bake pass.
        std::sync::Arc::new(crate::meshers::blocky::baked_library::BakedLibrary::default())
    }

    fn full_cube_side_surface(side: usize) -> SideSurface {
        let positions: Vec<Vector3f> = SIDE_CORNERS[side]
            .iter()
            .map(|&corner| CORNER_POSITION[corner])
            .collect();
        let indices = SIDE_QUAD_TRIANGLES[side].to_vec();
        let uvs = vec![
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

    fn full_cube_blocky_library(collision_enabled: bool) -> std::sync::Arc<BakedLibrary> {
        let air = BakedModel::default();
        let mut cube = BakedModel {
            empty: false,
            culls_neighbors: true,
            contributes_to_ao: true,
            ..Default::default()
        };
        cube.model.surface_count = 1;
        cube.model.surfaces[0].material_id = 0;
        cube.model.surfaces[0].collision_enabled = collision_enabled;
        for side in 0..Side::COUNT {
            cube.model.sides_surfaces[side][0] = full_cube_side_surface(side);
        }
        let mut library = BakedLibrary {
            models: vec![air, cube],
            indexed_materials_count: 1,
            ..Default::default()
        };
        crate::meshers::blocky::bake_library(&mut library);
        std::sync::Arc::new(library)
    }

    fn blocky_input_buffer() -> VoxelBuffer {
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(3));
        voxels.set_voxel(1, 1, 1, 1, ChannelId::Type.index());
        voxels
    }

    #[test]
    fn blocky_mesher_with_empty_library_emits_no_geometry() {
        let mut mesher = BlockyMesher::new(empty_blocky_library());
        // Solid block of voxel id 1 — but the library has no model for it,
        // so nothing gets emitted.
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(4));
        for z in 1..3 {
            for x in 1..3 {
                for y in 1..3 {
                    voxels.set_voxel(1, x, y, z, ChannelId::Type.index());
                }
            }
        }
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        assert_eq!(output.total_triangle_count(), 0);
    }

    #[test]
    fn blocky_mesher_padding_and_channels_match_constants() {
        let mesher = BlockyMesher::new(empty_blocky_library());
        assert_eq!(mesher.minimum_padding(), 1);
        assert_eq!(mesher.maximum_padding(), 1);
        assert_eq!(mesher.used_channels_mask(), 1u32 << ChannelId::Type.index());
    }

    #[test]
    fn blocky_collision_hint_emits_enabled_surface_geometry() {
        let mut mesher = BlockyMesher::new(full_cube_blocky_library(true));
        let voxels = blocky_input_buffer();
        let mut input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        input.collision_hint = true;

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        assert!(mesher.is_generating_collision_surface());
        assert!(output.total_triangle_count() > 0);
        assert_eq!(output.collision_surface.positions.len(), 24);
        assert_eq!(output.collision_surface.indices.len(), 36);
    }

    #[test]
    fn blocky_collision_hint_skips_surfaces_with_collision_disabled() {
        let mut mesher = BlockyMesher::new(full_cube_blocky_library(false));
        let voxels = blocky_input_buffer();
        let mut input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        input.collision_hint = true;

        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);

        assert!(mesher.is_generating_collision_surface());
        assert!(output.total_triangle_count() > 0);
        assert!(output.collision_surface.positions.is_empty());
        assert!(output.collision_surface.indices.is_empty());
    }

    #[test]
    fn blocky_mesher_reports_lod_unsupported_until_lod_inputs_are_used() {
        let mesher = BlockyMesher::new(empty_blocky_library());

        assert!(!mesher.supports_lod());
    }

    #[test]
    fn blocky_mesher_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BlockyMesher>();
    }
}
