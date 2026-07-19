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
use godot::classes::{ArrayMesh, INode3D, MeshInstance3D};
use godot::prelude::*;

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
    /// The engine-agnostic terrain core (lazy-initialised on first `_ready`).
    core: Option<VoxelTerrainCore>,
    /// Mesh block positions that have been uploaded to Godot MeshInstance3D
    /// children, keyed by `mesh_block_pos`.
    mesh_instances: HashMap<Vector3i, Gd<MeshInstance3D>>,
}

#[godot_api]
impl INode3D for VoxelTerrain {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            core: None,
            mesh_instances: HashMap::new(),
        }
    }

    fn ready(&mut self) {
        // Initialise the terrain core with a Waves generator + transvoxel mesher.
        // For MVP: generator-only (no stream save/load).
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

        let mesher = Arc::new(TransvoxelMesher::new());
        let meshing_dep = MeshingDependency::new(mesher, None);
        let core = VoxelTerrainCore::new_generator_only(data, meshing_dep);
        self.core = Some(core);
        godot_print!("VoxelTerrain ready — terrain core initialised");
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
        for lod in 0..core.lod_count() {
            for (&bpos, entry) in core.mesh_blocks_at_lod(lod).iter() {
                if !entry.is_loaded || self.mesh_instances.contains_key(&bpos) {
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

        // Upload after releasing the core borrow.
        for (bpos, verts, norms, idx, lod) in to_upload {
            self.upload_mesh(bpos, &verts, &norms, &idx, lod);
        }
    }
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
