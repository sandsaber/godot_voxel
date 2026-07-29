//! Additional Godot classes for voxel-core types.
//!
//! `VoxelBufferGD` exposes a VoxelBuffer as a Godot RefCounted.
//! `VoxelInstancerGD` is a Node3D for scatter-based instance placement.

use godot::prelude::*;
use voxel_core::instancing::scatter::{InstanceGenerator, RandomScatterGenerator};
use voxel_core::instancing::{InstanceLibrary, ScatterConfig};
use voxel_core::math::{Vector3f, Vector3i};
use voxel_core::storage::{VoxelBuffer, VoxelFormat};

// ---------------------------------------------------------------------------
// VoxelBufferGD — RefCounted wrapper around VoxelBuffer
// ---------------------------------------------------------------------------

/// A Godot `RefCounted` wrapping a [`VoxelBuffer`]. Exposes basic voxel
/// read/write to GDScript for testing and procedural generation.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelBufferGD {
    base: Base<RefCounted>,
    buffer: VoxelBuffer,
}

#[godot_api]
impl IRefCounted for VoxelBufferGD {
    fn init(base: Base<RefCounted>) -> Self {
        let mut buffer = VoxelBuffer::with_size(Vector3i::splat(1));
        VoxelFormat::new().configure_buffer(&mut buffer);
        Self { base, buffer }
    }
}

#[godot_api]
impl VoxelBufferGD {
    #[func]
    fn create(&mut self, size_x: i32, size_y: i32, size_z: i32) {
        self.buffer = VoxelBuffer::with_size(Vector3i::new(size_x, size_y, size_z));
        VoxelFormat::new().configure_buffer(&mut self.buffer);
    }

    #[func]
    fn set_voxel(&mut self, x: i32, y: i32, z: i32, channel: i32, value: i64) {
        self.buffer
            .set_voxel(value as u64, x, y, z, channel as usize);
    }

    #[func]
    fn get_voxel(&self, x: i32, y: i32, z: i32, channel: i32) -> i64 {
        self.buffer.get_voxel(x, y, z, channel as usize) as i64
    }

    #[func]
    fn get_size_x(&self) -> i32 {
        self.buffer.size().x
    }

    #[func]
    fn get_size_y(&self) -> i32 {
        self.buffer.size().y
    }

    #[func]
    fn get_size_z(&self) -> i32 {
        self.buffer.size().z
    }

    #[func]
    fn fill_channel(&mut self, channel: i32, value: i64) {
        self.buffer.fill(value as u64, channel as usize);
    }

    #[func]
    fn clear_channel(&mut self, channel: i32, value: i64) {
        self.buffer.clear_channel(channel as usize, value as u64);
    }
}

// ---------------------------------------------------------------------------
// VoxelInstancerGD — Node3D for scatter-based instance placement
// ---------------------------------------------------------------------------

/// A Godot `Node3D` that scatters instances (trees, rocks, grass) on
/// a parent [`VoxelTerrain`](crate::terrain::VoxelTerrain) using
/// [`voxel_core::instancing`].
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelInstancerGD {
    base: Base<Node3D>,
    /// Instance library (items to scatter).
    library: InstanceLibrary,
    /// Scatter config.
    config: ScatterConfig,
    /// Density multiplier.
    #[var]
    density_multiplier: f32,
}

#[godot_api]
impl INode3D for VoxelInstancerGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            library: InstanceLibrary::new(),
            config: ScatterConfig::default(),
            density_multiplier: 1.0,
        }
    }

    fn ready(&mut self) {
        godot_print!("VoxelInstancerGD ready");
    }
}

#[godot_api]
impl VoxelInstancerGD {
    /// Add a scatter item (returns item index).
    #[func]
    fn add_item(&mut self, name: GString, density: f64, min_scale: f64, max_scale: f64) -> i32 {
        let item = voxel_core::instancing::InstanceLibraryItem {
            name: name.to_string(),
            density: density as f32,
            min_scale: min_scale as f32,
            max_scale: max_scale as f32,
            ..Default::default()
        };
        self.library.add_item(item) as i32
    }

    /// Get the number of items in the library.
    #[func]
    fn get_item_count(&self) -> i32 {
        self.library.len() as i32
    }

    /// Set the random seed for scatter.
    #[func]
    fn set_seed(&mut self, seed: i64) {
        self.config.seed = seed as u32;
    }

    /// Generate instances from a VoxelBufferGD's surface.
    /// Extracts surface points where solid meets air, runs the scatter
    /// generator for each library item, returns total instance count.
    #[func]
    fn scatter_from_buffer(&mut self, buffer: Gd<RefCounted>) -> i32 {
        if self.library.is_empty() {
            return 0;
        }
        // Try to cast to VoxelBufferGD for direct field access.
        if let Ok(buf_gd) = buffer.clone().try_cast::<VoxelBufferGD>() {
            let bound = buf_gd.bind();
            let sx = bound.get_size_x();
            let sy = bound.get_size_y();
            let sz = bound.get_size_z();

            let mut positions = Vec::new();
            let mut normals = Vec::new();
            for z in 1..sz {
                for y in 1..sy {
                    for x in 1..sx {
                        let vt = bound.get_voxel(x, y, z, 0);
                        let vt_below = bound.get_voxel(x, y - 1, z, 0);
                        if vt != 0 && vt_below == 0 {
                            positions.push(Vector3f::new(x as f32, y as f32, z as f32));
                            normals.push(Vector3f::new(0.0, 1.0, 0.0));
                        }
                    }
                }
            }
            drop(bound);
            drop(buf_gd);

            if positions.is_empty() {
                return 0;
            }

            let mut total = 0;
            for (idx, item) in self.library.items.iter().enumerate() {
                let gen = RandomScatterGenerator {
                    density: item.density * self.density_multiplier,
                    min_scale: item.min_scale,
                    max_scale: item.max_scale,
                    snap_to_normal: item.snap_to_normal,
                };
                let result = gen.generate(&positions, &normals, idx as u32, &self.config);
                total += result.len();
            }
            return total as i32;
        }
        0
    }

    /// Generate instances from dummy surface positions (test/debug).
    /// Returns the total instance count.
    #[func]
    fn scatter_test(&self, count: i32) -> i32 {
        if self.library.is_empty() {
            return 0;
        }
        let positions: Vec<Vector3f> = (0..count)
            .map(|i| Vector3f::new(i as f32, 0.0, 0.0))
            .collect();
        let normals = vec![Vector3f::new(0.0, 1.0, 0.0); count as usize];
        let gen = RandomScatterGenerator {
            density: self.library.items[0].density * self.density_multiplier,
            min_scale: self.library.items[0].min_scale,
            max_scale: self.library.items[0].max_scale,
            snap_to_normal: true,
        };
        let result = gen.generate(&positions, &normals, 0, &self.config);
        result.len() as i32
    }
}

// ---------------------------------------------------------------------------
// VoxelToolTerrainGD — RefCounted terrain editing tool
// ---------------------------------------------------------------------------

/// A Godot `RefCounted` that wraps a reference to a [`VoxelTerrain`](crate::terrain::VoxelTerrain)
/// for GDScript-callable terrain editing operations.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelToolTerrainGD {
    base: Base<RefCounted>,
    /// Weak reference to the terrain node path.
    terrain_path: GString,
}

#[godot_api]
impl IRefCounted for VoxelToolTerrainGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            terrain_path: "..".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelToolTerrainGD {
    #[func]
    fn set_terrain_path(&mut self, path: GString) {
        self.terrain_path = path;
    }

    #[func]
    fn get_terrain_path(&self) -> GString {
        self.terrain_path.clone()
    }
}

// ---------------------------------------------------------------------------
// VoxelRaycastResultGD — RefCounted result container
// ---------------------------------------------------------------------------

/// Result of a voxel raycast. Contains hit position, previous position,
/// and distance along the ray.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelRaycastResultGD {
    base: Base<RefCounted>,
    #[var]
    hit_x: i32,
    #[var]
    hit_y: i32,
    #[var]
    hit_z: i32,
    #[var]
    prev_x: i32,
    #[var]
    prev_y: i32,
    #[var]
    prev_z: i32,
    #[var]
    distance: f32,
    #[var]
    normal_x: i32,
    #[var]
    normal_y: i32,
    #[var]
    normal_z: i32,
}

#[godot_api]
impl IRefCounted for VoxelRaycastResultGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            hit_x: 0,
            hit_y: 0,
            hit_z: 0,
            prev_x: 0,
            prev_y: 0,
            prev_z: 0,
            distance: 0.0,
            normal_x: 0,
            normal_y: 0,
            normal_z: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelNodeGD — base Node3D for voxel volumes (VoxelNode equivalent)
// ---------------------------------------------------------------------------

/// Base Node3D for voxel volume nodes. Holds shared properties.
/// In C++ this is the base class for VoxelTerrain/VoxelLodTerrain.
/// In Rust, VoxelTerrain inherits Node3D directly, but this class
/// exists for API parity and future VoxelLodTerrain.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelNodeGD {
    base: Base<Node3D>,
    /// Whether the terrain streams blocks around viewers.
    #[var]
    auto_load: bool,
    /// Maximum view distance in voxels.
    #[var]
    max_view_distance: i64,
}

#[godot_api]
impl INode3D for VoxelNodeGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            auto_load: true,
            max_view_distance: 192,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelGeneratorGraphGD — Resource wrapper for GraphGenerator
// ---------------------------------------------------------------------------

/// A Godot `Resource` wrapping a graph-based terrain generator.
/// In a full implementation this would expose the graph node editor.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorGraphGD {
    base: Base<Resource>,
    /// Graph nodes serialized as a JSON string (for save/load).
    graph_json: GString,
}

#[godot_api]
impl IResource for VoxelGeneratorGraphGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            graph_json: "{}".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelGeneratorGraphGD {
    #[func]
    fn get_graph_json(&self) -> GString {
        self.graph_json.clone()
    }

    #[func]
    fn set_graph_json(&mut self, json: GString) {
        self.graph_json = json;
    }
}
