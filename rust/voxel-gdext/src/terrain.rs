//! Godot terrain nodes wrapping `voxel_core::terrain::VoxelTerrainCore`.
//!
//! `VoxelTerrain` is a `Node3D` that owns a `VoxelTerrainCore` (the
//! engine-agnostic paging orchestrator). Each `_process` tick it feeds viewer
//! positions into the core, drains mesh outputs, and uploads them as
//! `ArrayMesh` instances into child `MeshInstance3D` nodes — producing a
//! visible terrain in the Godot editor and at runtime.

use std::collections::HashMap;
use std::sync::Arc;

use godot::classes::mesh::PrimitiveType;
use godot::classes::{ArrayMesh, INode3D, Material, MeshInstance3D};
use godot::prelude::*;

use voxel_core::constants::voxel_constants::MAX_LOD;
use voxel_core::engine::MeshingDependency;
use voxel_core::math::Vector3i;
use voxel_core::meshers::TransvoxelMesher;
use voxel_core::storage::{ChannelDepth, ChannelId, VoxelData, VoxelFormat};
use voxel_core::terrain::{ViewerUpdate, VoxelTerrainCore};

// ---------------------------------------------------------------------------
// VoxelTerrain
// ---------------------------------------------------------------------------

/// A Godot `Node3D` that renders voxel terrain. Wraps
/// [`voxel_core::terrain::VoxelTerrainCore`] — the engine-agnostic paging
/// orchestrator that loads data blocks, meshes them with the transvoxel
/// mesher, and manages LOD + view/unview based on paired
/// [`VoxelViewer`](self::VoxelViewer) positions.
///
/// In GDScript: add a `VoxelTerrain` node to the scene tree, then add a
/// `VoxelViewer` child (or sibling). The terrain will page in around the viewer.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelTerrain {
    base: Base<Node3D>,
    core: Option<VoxelTerrainCore>,
    mesh_instances: HashMap<Vector3i, Gd<MeshInstance3D>>,
    generator_resource: Option<Gd<Resource>>,
    #[export]
    #[var(get = get_stream, set = set_stream)]
    stream: PhantomVar<Option<Gd<Resource>>>,
    stream_resource: Option<Gd<Resource>>,
    lod_count: u8,
    dirty_blocks: std::collections::HashSet<Vector3i>,
    /// Optional material override applied to all mesh blocks.
    material_override: Option<Gd<Material>>,
    /// Whether to generate collision shapes for mesh blocks.
    generate_collision: bool,
}

#[godot_api]
impl INode3D for VoxelTerrain {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            core: None,
            mesh_instances: HashMap::new(),
            generator_resource: None,
            stream: Default::default(),
            stream_resource: None,
            lod_count: 1,
            dirty_blocks: std::collections::HashSet::new(),
            material_override: None,
            generate_collision: false,
        }
    }

    fn ready(&mut self) {
        let mut data = VoxelData::new();
        data.set_bounds(voxel_core::math::Box3i::new(
            Vector3i::splat(-512),
            Vector3i::splat(2048),
        ));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        data.set_format(format);

        let generator = self.resolve_generator();
        data.set_generator(Some(generator));

        let mesher = Arc::new(TransvoxelMesher::new());
        let meshing_dep = MeshingDependency::new(mesher, None);
        let stream_was_assigned = self.stream_resource.is_some();
        let explicit_stream = self
            .stream_resource
            .clone()
            .and_then(crate::streams::resolve_core_stream);
        if stream_was_assigned && explicit_stream.is_none() {
            godot_error!("VoxelTerrain.stream must be VoxelStreamMemory or VoxelStreamRegionFiles");
        }
        let has_explicit_stream = explicit_stream.is_some();
        let selected_stream = select_terrain_stream(explicit_stream, self.lod_count);

        let core = match selected_stream {
            Some(stream) => {
                if has_explicit_stream {
                    data.set_streaming_enabled(true);
                    data.set_full_load_completed(false);
                }
                VoxelTerrainCore::new_with_lod_count(data, stream, meshing_dep, self.lod_count)
            }
            None => VoxelTerrainCore::new_generator_only(data, meshing_dep),
        };
        self.core = Some(core);
        godot_print!(
            "VoxelTerrain ready — terrain core initialised (lod_count={})",
            self.lod_count
        );
    }

    fn process(&mut self, _delta: f64) {
        // Collect viewer updates BEFORE borrowing core (avoids borrow conflict).
        let mut viewers = Vec::new();
        let mut id = 1u32;
        for child in self.base().get_children().iter_shared() {
            if let Ok(viewer) = child.try_cast::<VoxelViewer>() {
                let pos = viewer.bind().get_world_position();
                viewers.push(ViewerUpdate {
                    id,
                    world_position_voxels: Vector3i::new(pos.x as i32, pos.y as i32, pos.z as i32),
                    horizontal_view_distance_voxels: viewer.bind().view_distance as i32,
                    vertical_view_distance_voxels: viewer.bind().view_distance as i32,
                    requires_meshes: true,
                });
                id += 1;
            }
        }

        let Some(core) = self.core.as_mut() else {
            return;
        };

        // Run the paging tick.
        let _events = core.process(&viewers);

        // Collect mesh data to upload (collect first to avoid borrow conflict).
        /// Pending mesh upload: (position, vertices, normals, indices, lod)
        type PendingMesh = (Vector3i, Vec<f32>, Vec<f32>, Vec<i32>, u8);
        let mut to_upload: Vec<PendingMesh> = Vec::new();
        let dirty = std::mem::take(&mut self.dirty_blocks);
        for lod in 0..core.lod_count() {
            for (&bpos, entry) in core.mesh_blocks_at_lod(lod).iter() {
                if !entry.is_loaded {
                    continue;
                }
                // Upload if: (a) not yet uploaded, or (b) dirty (edited).
                let is_dirty = dirty.contains(&bpos);
                if !is_dirty && self.mesh_instances.contains_key(&bpos) {
                    continue;
                }
                if let Some(output) = &entry.output {
                    if let Some(surface) = output.surfaces.first() {
                        if let voxel_core::meshers::SurfaceArrays::Transvoxel(arrays) =
                            &surface.arrays
                        {
                            if !arrays.indices.is_empty() {
                                let verts: Vec<f32> = arrays
                                    .vertices
                                    .iter()
                                    .flat_map(|v| [v.x, v.y, v.z])
                                    .collect();
                                let norms: Vec<f32> = arrays
                                    .normals
                                    .iter()
                                    .flat_map(|n| [n.x, n.y, n.z])
                                    .collect();
                                let idx: Vec<i32> = arrays.indices.to_vec();
                                to_upload.push((bpos, verts, norms, idx, lod));
                            }
                        }
                    }
                }
            }
        }

        // Upload after releasing the core borrow. For dirty blocks, remove old
        // MeshInstance3D first so we replace it with the new mesh.
        for (bpos, verts, norms, idx, lod) in to_upload {
            if let Some(mut old) = self.mesh_instances.remove(&bpos) {
                old.queue_free();
            }
            self.upload_mesh(bpos, &verts, &norms, &idx, lod);
        }
    }
}

#[cfg(test)]
mod stream_selection_tests {
    use super::*;
    use voxel_core::constants::voxel_constants::MAX_LOD;
    use voxel_core::streams::{MemoryStream, VoxelStream};

    #[test]
    fn lod_count_is_clamped_before_narrowing_to_u8() {
        assert_eq!(clamp_lod_count(0), 1);
        assert_eq!(clamp_lod_count(-1), 1);
        assert_eq!(clamp_lod_count(256), MAX_LOD as u8);
        assert_eq!(clamp_lod_count(MAX_LOD as i32), MAX_LOD as u8);
    }

    #[test]
    fn explicit_stream_wins_and_only_multi_lod_gets_an_internal_fallback() {
        let explicit: Arc<dyn VoxelStream> = Arc::new(MemoryStream::new());
        let selected = select_terrain_stream(Some(explicit.clone()), 1).unwrap();
        assert!(Arc::ptr_eq(&selected, &explicit));
        assert!(select_terrain_stream(None, 1).is_none());
        assert!(select_terrain_stream(None, 2).is_some());
    }
}

fn select_terrain_stream(
    explicit: Option<Arc<dyn voxel_core::streams::VoxelStream>>,
    lod_count: u8,
) -> Option<Arc<dyn voxel_core::streams::VoxelStream>> {
    explicit.or_else(|| {
        (lod_count > 1).then(|| {
            Arc::new(voxel_core::streams::MemoryStream::new())
                as Arc<dyn voxel_core::streams::VoxelStream>
        })
    })
}

fn clamp_lod_count(count: i32) -> u8 {
    count.clamp(1, MAX_LOD as i32) as u8
}

#[godot_api]
impl VoxelTerrain {
    /// Returns the number of loaded mesh blocks (all LODs).
    #[func]
    fn get_mesh_block_count(&self) -> i32 {
        self.mesh_instances.len() as i32
    }

    /// Returns the voxel-core version string (diagnostic).
    #[func]
    fn get_version(&self) -> GString {
        voxel_core::VERSION.to_godot()
    }

    /// The generator resource (VoxelGeneratorWaves or VoxelGeneratorFlat).
    /// Set this in the inspector to choose the terrain shape.
    #[func]
    fn get_generator(&self) -> Variant {
        match &self.generator_resource {
            Some(g) => g.to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn set_generator(&mut self, value: Gd<Resource>) {
        self.generator_resource = Some(value);
    }

    #[func]
    fn get_stream(&self) -> Option<Gd<Resource>> {
        self.stream_resource.clone()
    }

    #[func]
    fn set_stream(&mut self, value: Option<Gd<Resource>>) {
        self.stream_resource = value;
    }

    /// Number of LOD levels (1 = single-LOD, 2+ = multi-LOD with transitions).
    /// Must be set before _ready (in the inspector or via script before adding
    /// to the scene tree).
    #[func]
    fn get_lod_count(&self) -> i32 {
        self.lod_count as i32
    }

    #[func]
    fn set_lod_count(&mut self, count: i32) {
        self.lod_count = clamp_lod_count(count);
    }

    /// Material override applied to all terrain mesh blocks.
    #[func]
    fn get_material_override(&self) -> Variant {
        match &self.material_override {
            Some(m) => m.to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn set_material_override(&mut self, value: Gd<Material>) {
        self.material_override = Some(value);
    }

    /// Whether to generate trimesh collision for terrain blocks.
    #[func]
    fn get_generate_collision(&self) -> bool {
        self.generate_collision
    }

    #[func]
    fn set_generate_collision(&mut self, enabled: bool) {
        self.generate_collision = enabled;
    }

    /// Set a voxel's SDF value at world position. Triggers a re-mesh of the
    /// affected block on the next process tick.
    #[func]
    fn set_voxel_sdf(&mut self, world_x: i32, world_y: i32, world_z: i32, value: f32) -> bool {
        let Some(core) = self.core.as_ref() else {
            return false;
        };
        let pos = Vector3i::new(world_x, world_y, world_z);
        let data = core.data();
        let channel = ChannelId::Sdf.index();
        let settings = data.settings_snapshot();
        let raw = voxel_core::storage::voxel_buffer::real_to_raw_voxel(
            value,
            settings.format.depths[channel],
        );
        let ok = core.try_edit_voxel(raw, pos, channel);
        if ok {
            // Mark the block dirty so the process loop re-uploads its mesh.
            let block_pos = voxel_core::storage::voxel_data_map::VoxelDataMap::voxel_to_block_b(
                pos,
                data.block_size_po2(),
            );
            self.dirty_blocks.insert(block_pos);
        }
        ok
    }

    /// Get a voxel's SDF value at world position.
    #[func]
    fn get_voxel_sdf(&self, world_x: i32, world_y: i32, world_z: i32) -> f32 {
        let Some(core) = self.core.as_ref() else {
            return 0.0;
        };
        let pos = Vector3i::new(world_x, world_y, world_z);
        let data = core.data();
        let channel = ChannelId::Sdf.index();
        // SharedVoxelData doesn't expose get_voxel directly; use the settings
        // default if no block is loaded. This is a read-only diagnostic.
        let settings = data.settings_snapshot();
        let block_pos = voxel_core::storage::voxel_data_map::VoxelDataMap::voxel_to_block_b(
            pos,
            data.block_size_po2(),
        );
        let raw = data.with_lod_map(0, |map| {
            map.get_block(block_pos)
                .filter(|b| b.has_voxels())
                .map(|b| {
                    b.voxels().get_voxel(
                        pos.x.rem_euclid(data.block_size() as i32),
                        pos.y.rem_euclid(data.block_size() as i32),
                        pos.z.rem_euclid(data.block_size() as i32),
                        channel,
                    )
                })
                .unwrap_or(0)
        });
        voxel_core::storage::voxel_buffer::raw_voxel_to_real(raw, settings.format.depths[channel])
    }

    /// Returns the terrain bounds as [min_x, min_y, min_z, size_x, size_y, size_z].
    #[func]
    fn get_bounds(&self) -> PackedInt32Array {
        if let Some(core) = self.core.as_ref() {
            let bounds = core.data().bounds();
            PackedInt32Array::from(&[
                bounds.position.x,
                bounds.position.y,
                bounds.position.z,
                bounds.size.x,
                bounds.size.y,
                bounds.size.z,
            ])
        } else {
            PackedInt32Array::new()
        }
    }

    /// SDF raycast: march along a ray from `origin` in `direction` (normalized)
    /// up to `max_distance` voxels. Returns the hit position as
    /// `[x, y, z, hit]` where `hit` is 1.0 if the ray hit terrain, 0.0 otherwise.
    /// Uses a simple fixed-step SDF march (no spatial acceleration — MVP).
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn raycast(
        &self,
        origin_x: f64,
        origin_y: f64,
        origin_z: f64,
        dir_x: f64,
        dir_y: f64,
        dir_z: f64,
        max_distance: f64,
    ) -> PackedFloat32Array {
        let Some(core) = self.core.as_ref() else {
            return PackedFloat32Array::new();
        };
        let data = core.data();
        let channel = ChannelId::Sdf.index();
        let settings = data.settings_snapshot();
        let depth = settings.format.depths[channel];
        let block_size_po2 = data.block_size_po2();

        let ox = origin_x as f32;
        let oy = origin_y as f32;
        let oz = origin_z as f32;
        let dx = dir_x as f32;
        let dy = dir_y as f32;
        let dz = dir_z as f32;
        let max_d = max_distance as f32;

        // Normalise direction.
        let dlen = (dx * dx + dy * dy + dz * dz).sqrt();
        if dlen < 1e-6 {
            return PackedFloat32Array::from(&[0.0, 0.0, 0.0, 0.0]);
        }
        let ndx = dx / dlen;
        let ndy = dy / dlen;
        let ndz = dz / dlen;

        // March with 1-voxel steps.
        let step = 1.0f32;
        let mut t = 0.0f32;
        while t < max_d {
            let px = ox + ndx * t;
            let py = oy + ndy * t;
            let pz = oz + ndz * t;
            let vi = Vector3i::new(px as i32, py as i32, pz as i32);
            let block_pos = voxel_core::storage::voxel_data_map::VoxelDataMap::voxel_to_block_b(
                vi,
                block_size_po2,
            );
            let raw = data.with_lod_map(0, |map| {
                map.get_block(block_pos)
                    .filter(|b| b.has_voxels())
                    .map(|b| {
                        b.voxels().get_voxel(
                            vi.x.rem_euclid(data.block_size() as i32),
                            vi.y.rem_euclid(data.block_size() as i32),
                            vi.z.rem_euclid(data.block_size() as i32),
                            channel,
                        )
                    })
                    .unwrap_or(0)
            });
            let sdf = voxel_core::storage::voxel_buffer::raw_voxel_to_real(raw, depth);
            // SDF < 0 means inside solid → hit.
            if sdf < 0.0 {
                return PackedFloat32Array::from(&[px, py, pz, 1.0]);
            }
            t += step;
        }
        PackedFloat32Array::from(&[0.0, 0.0, 0.0, 0.0])
    }
}

impl VoxelTerrain {
    /// Resolve the Godot generator resource into a voxel-core generator.
    /// If no resource is set, defaults to Waves(60, 128).
    fn resolve_generator(&self) -> voxel_core::storage::SharedVoxelGenerator {
        use crate::generators::{
            VoxelGeneratorFlat, VoxelGeneratorHeightmap, VoxelGeneratorNoise, VoxelGeneratorWaves,
        };

        if let Some(res) = &self.generator_resource {
            if let Ok(waves) = res.clone().try_cast::<VoxelGeneratorWaves>() {
                return waves.bind().create_core_generator();
            }
            if let Ok(flat) = res.clone().try_cast::<VoxelGeneratorFlat>() {
                return flat.bind().create_core_generator();
            }
            if let Ok(noise) = res.clone().try_cast::<VoxelGeneratorNoise>() {
                return noise.bind().create_core_generator();
            }
            if let Ok(hm) = res.clone().try_cast::<VoxelGeneratorHeightmap>() {
                return hm.bind().create_core_generator();
            }
        }
        // Default: Waves with sensible parameters.
        let mut waves = voxel_core::generators::simple::Waves::default();
        waves.set_pattern_size(voxel_core::math::Vector2f::new(128.0, 128.0));
        waves.heightmap.height_range = 60.0;
        Arc::new(waves)
    }
}

impl VoxelTerrain {
    /// Upload a transvoxel mesh array as an `ArrayMesh` into a child
    /// `MeshInstance3D` node positioned at the block's world origin.
    fn upload_mesh(&mut self, bpos: Vector3i, verts: &[f32], norms: &[f32], idx: &[i32], lod: u8) {
        let mut mesh_arrays = Array::new();

        // Positions (PackedVector3Array).
        let positions: Vec<Vector3> = verts
            .chunks_exact(3)
            .map(|c| Vector3::new(c[0], c[1], c[2]))
            .collect();
        mesh_arrays.push(&PackedVector3Array::from(positions.as_slice()));

        // Normals.
        let normals: Vec<Vector3> = norms
            .chunks_exact(3)
            .map(|c| Vector3::new(c[0], c[1], c[2]))
            .collect();
        mesh_arrays.push(&PackedVector3Array::from(normals.as_slice()));

        // Empty UV, UV2, color, etc. (indices 2..11).
        for _ in 2..12 {
            mesh_arrays.push(&Variant::nil());
        }

        // Indices (PackedInt32Array).
        mesh_arrays.push(&PackedInt32Array::from(idx));

        // Create ArrayMesh.
        let mut array_mesh = ArrayMesh::new_gd();
        let block_size = 16i32;
        let lod_stride = 1i32 << lod;
        let origin = Vector3::new(
            (bpos.x * block_size * lod_stride) as f32,
            (bpos.y * block_size * lod_stride) as f32,
            (bpos.z * block_size * lod_stride) as f32,
        );
        array_mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &mesh_arrays);

        // Create MeshInstance3D child.
        let mut instance = MeshInstance3D::new_alloc();
        instance.set_mesh(&array_mesh);
        instance.set_position(origin);
        // Apply material override if set.
        if let Some(mat) = &self.material_override {
            instance.set_material_override(mat);
        }
        // Generate collision if enabled.
        if self.generate_collision {
            instance.create_trimesh_collision();
        }
        let instance_name = format!("mesh_{}_{}_{}", bpos.x, bpos.y, bpos.z);
        instance.set_name(&instance_name);
        self.base_mut().add_child(&instance);

        self.mesh_instances.insert(bpos, instance);
    }
}

// ---------------------------------------------------------------------------
// VoxelViewer
// ---------------------------------------------------------------------------

/// A Godot `Node3D` that marks a viewer position for the terrain paging system.
/// Add as a child of (or sibling to) a [`VoxelTerrain`](self::VoxelTerrain).
///
/// The terrain pages blocks around each viewer's world position within
/// `view_distance` voxels.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelViewer {
    base: Base<Node3D>,
    /// View distance in voxels (horizontal and vertical).
    #[var]
    view_distance: i64,
}

#[godot_api]
impl INode3D for VoxelViewer {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            view_distance: 96,
        }
    }

    fn ready(&mut self) {
        godot_print!("VoxelViewer ready — view_distance={}", self.view_distance);
    }
}

impl VoxelViewer {
    /// Get the viewer's world position as a `Vector3` (f32).
    fn get_world_position(&self) -> Vector3 {
        self.base().get_global_position()
    }
}
