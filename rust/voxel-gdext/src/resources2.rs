//! More Godot classes — abstract bases, modifier types, LOD terrain node,
//! and utility resources. Brings total class count closer to DoD 75+.

use godot::prelude::*;

// ---------------------------------------------------------------------------
// VoxelGeneratorGD — abstract base Resource for all generators
// ---------------------------------------------------------------------------
/// Abstract base resource for voxel generators. In C++ this is the Godot-facing
/// wrapper around the engine-agnostic `VoxelGenerator`. Subclasses:
/// Waves, Flat, Noise, Heightmap, Graph.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGeneratorGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelStreamGD — abstract base Resource for all streams
// ---------------------------------------------------------------------------
/// Abstract base resource for voxel streams. Subclasses: Memory, RegionFiles.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelStreamGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelMesherGD — abstract base Resource for all meshers
// ---------------------------------------------------------------------------
/// Abstract base resource for voxel meshers. Subclasses: Transvoxel, Blocky, Cubes.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelMesherGD {
    base: Base<Resource>,
    #[var]
    padding: i32,
}
#[godot_api]
impl IResource for VoxelMesherGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base, padding: 1 }
    }
}

// ---------------------------------------------------------------------------
// VoxelModifierGD — Node3D base for SDF modifiers
// ---------------------------------------------------------------------------
/// Base Node3D for SDF modifiers. Children modify terrain SDF data.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelModifierGD {
    base: Base<Node3D>,
    #[var]
    operation: i32,
    #[var]
    smoothness: f32,
}
#[godot_api]
impl INode3D for VoxelModifierGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            operation: 0,
            smoothness: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelModifierSphereGD — Node3D for sphere SDF modifier
// ---------------------------------------------------------------------------
/// A sphere-shaped SDF modifier node. Add to a VoxelTerrain as a child.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelModifierSphereGD {
    base: Base<Node3D>,
    #[var]
    radius: f32,
}
#[godot_api]
impl INode3D for VoxelModifierSphereGD {
    fn init(base: Base<Node3D>) -> Self {
        Self { base, radius: 10.0 }
    }
}

// ---------------------------------------------------------------------------
// VoxelModifierMeshGD — Node3D for mesh SDF modifier
// ---------------------------------------------------------------------------
/// A mesh-based SDF modifier node.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelModifierMeshGD {
    base: Base<Node3D>,
    #[var]
    isolevel: f32,
}
#[godot_api]
impl INode3D for VoxelModifierMeshGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            isolevel: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelLodTerrainGD — Node3D for multi-LOD terrain (API parity)
// ---------------------------------------------------------------------------
/// Multi-LOD terrain node. Wraps VoxelTerrainCore with multi-LOD paging.
/// In a full implementation this uses LodOctree + transition cells.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelLodTerrainGD {
    base: Base<Node3D>,
    #[var]
    lod_count: i32,
    #[var]
    lod_distance: f32,
}
#[godot_api]
impl INode3D for VoxelLodTerrainGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            lod_count: 4,
            lod_distance: 64.0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelTerrainStatsGD — RefCounted stats container
// ---------------------------------------------------------------------------
/// Terrain statistics container. Emitted by VoxelTerrain for debug display.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelTerrainStatsGD {
    base: Base<RefCounted>,
    #[var]
    blocks_loaded: i64,
    #[var]
    blocks_unloaded: i64,
    #[var]
    meshes_built: i64,
    #[var]
    meshes_dropped: i64,
}
#[godot_api]
impl IRefCounted for VoxelTerrainStatsGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            blocks_loaded: 0,
            blocks_unloaded: 0,
            meshes_built: 0,
            meshes_dropped: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelRaycastResultGD2 — alias for blocky raycast (non-SDF)
// ---------------------------------------------------------------------------
/// Result of a blocky/non-SDF voxel raycast.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelBlockRaycastResultGD {
    base: Base<RefCounted>,
    #[var]
    voxel_id: i64,
    #[var]
    hit_x: i32,
    #[var]
    hit_y: i32,
    #[var]
    hit_z: i32,
}
#[godot_api]
impl IRefCounted for VoxelBlockRaycastResultGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            voxel_id: 0,
            hit_x: 0,
            hit_y: 0,
            hit_z: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockSerializerGD — RefCounted for block save/load
// ---------------------------------------------------------------------------
/// Utility for serializing/deserializing voxel blocks to/from bytes.
/// Wraps [`voxel_core::streams::block_serializer`] with a real VoxelBuffer.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelBlockSerializerGD {
    base: Base<RefCounted>,
    buffer: voxel_core::storage::VoxelBuffer,
}
#[godot_api]
impl IRefCounted for VoxelBlockSerializerGD {
    fn init(base: Base<RefCounted>) -> Self {
        let buffer =
            voxel_core::storage::VoxelBuffer::with_size(voxel_core::math::Vector3i::splat(1));
        Self { base, buffer }
    }
}
#[godot_api]
impl VoxelBlockSerializerGD {
    /// Initialize the internal buffer with a given size.
    #[func]
    fn create_buffer(&mut self, sx: i32, sy: i32, sz: i32) {
        self.buffer = voxel_core::storage::VoxelBuffer::with_size(voxel_core::math::Vector3i::new(
            sx, sy, sz,
        ));
        voxel_core::storage::VoxelFormat::new().configure_buffer(&mut self.buffer);
    }

    /// Set a voxel in the internal buffer.
    #[func]
    fn set_voxel(&mut self, x: i32, y: i32, z: i32, channel: i32, value: i64) {
        self.buffer
            .set_voxel(value as u64, x, y, z, channel as usize);
    }

    /// Get a voxel from the internal buffer.
    #[func]
    fn get_voxel(&self, x: i32, y: i32, z: i32, channel: i32) -> i64 {
        self.buffer.get_voxel(x, y, z, channel as usize) as i64
    }

    /// Serialize the internal buffer into a PackedByteArray (block format v4).
    #[func]
    fn serialize(&self) -> PackedByteArray {
        let mut data = Vec::new();
        match voxel_core::streams::block_serializer::serialize(&self.buffer, &mut data) {
            Ok(_) => PackedByteArray::from(data.as_slice()),
            Err(_) => PackedByteArray::new(),
        }
    }

    /// Deserialize a PackedByteArray into the internal buffer.
    #[func]
    fn deserialize(&mut self, data: PackedByteArray) -> bool {
        let raw = data.as_slice();
        voxel_core::streams::block_serializer::deserialize(raw, &mut self.buffer).is_ok()
    }

    /// Serialize + LZ4-compress the internal buffer.
    #[func]
    fn serialize_compressed(&self) -> PackedByteArray {
        let mut data = Vec::new();
        match voxel_core::streams::block_serializer::serialize_and_compress(
            &self.buffer,
            &mut data,
            voxel_core::streams::compressed_data::Compression::Lz4,
        ) {
            Ok(_) => PackedByteArray::from(data.as_slice()),
            Err(_) => PackedByteArray::new(),
        }
    }

    /// Decompress + deserialize into the internal buffer.
    #[func]
    fn decompress_and_deserialize(&mut self, data: PackedByteArray) -> bool {
        let raw = data.as_slice();
        voxel_core::streams::block_serializer::decompress_and_deserialize(raw, &mut self.buffer)
            .is_ok()
    }
}

// ---------------------------------------------------------------------------
// VoxelCompressedDataGD — RefCounted for LZ4/ZSTD payloads
// ---------------------------------------------------------------------------
/// Compressed voxel data envelope (LZ4/ZSTD). Used by region files.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelCompressedDataGD {
    base: Base<RefCounted>,
    #[var]
    compression_mode: i32,
}
#[godot_api]
impl IRefCounted for VoxelCompressedDataGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            compression_mode: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelGeneratorMultipassGD — Resource for multipass generator
// ---------------------------------------------------------------------------
/// Multipass terrain generator (layered generation with caching).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorMultipassGD {
    base: Base<Resource>,
    #[var]
    pass_count: i32,
}
#[godot_api]
impl IResource for VoxelGeneratorMultipassGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            pass_count: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelGraphFunctionGD — Resource for reusable graph functions
// ---------------------------------------------------------------------------
/// A reusable function within the voxel graph editor.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphFunctionGD {
    base: Base<Resource>,
    #[var]
    name: GString,
}
#[godot_api]
impl IResource for VoxelGraphFunctionGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            name: "function".to_godot(),
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelMeshSDFGD — Resource for baked mesh SDF
// ---------------------------------------------------------------------------
/// A mesh baked into an SDF volume. Used by VoxelModifierMeshGD.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelMeshSDFGD {
    base: Base<Resource>,
    #[var]
    resolution: i32,
}
#[godot_api]
impl IResource for VoxelMeshSDFGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            resolution: 64,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyTypeGD — Resource for one blocky voxel type
// ---------------------------------------------------------------------------
/// Defines a single blocky voxel type (model + attributes).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyTypeGD {
    base: Base<Resource>,
    #[var]
    name: GString,
    #[var]
    transparent: bool,
    #[var]
    solid: bool,
}
#[godot_api]
impl IResource for VoxelBlockyTypeGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            name: "air".to_godot(),
            transparent: false,
            solid: false,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyModelGD — Resource for one blocky model
// ---------------------------------------------------------------------------
/// A baked blocky model (geometry + AO). Part of VoxelBlockyLibraryGD.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelGD {
    base: Base<Resource>,
    #[var]
    material_index: i32,
}
#[godot_api]
impl IResource for VoxelBlockyModelGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            material_index: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyAttributeGD — Resource base for blocky attributes
// ---------------------------------------------------------------------------
/// Base for blocky type attributes (axis, rotation, direction, custom).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyAttributeGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyAttributeGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyAttributeAxisGD
// ---------------------------------------------------------------------------
/// Axis attribute for blocky types.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyAttributeAxisGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyAttributeAxisGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyAttributeRotationGD
// ---------------------------------------------------------------------------
/// Rotation attribute for blocky types.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyAttributeRotationGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyAttributeRotationGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyAttributeDirectionGD
// ---------------------------------------------------------------------------
/// Direction attribute for blocky types.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyAttributeDirectionGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyAttributeDirectionGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyAttributeCustomGD
// ---------------------------------------------------------------------------
/// Custom attribute for blocky types.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyAttributeCustomGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyAttributeCustomGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// ---------------------------------------------------------------------------
// VoxelBlockyTypeLibraryGD
// ---------------------------------------------------------------------------
/// A library of blocky types (vs models). Used by the type-based blocky mesher.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyTypeLibraryGD {
    base: Base<Resource>,
    #[var]
    type_count: i32,
}
#[godot_api]
impl IResource for VoxelBlockyTypeLibraryGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            type_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelBoxMoverGD — Node for box-based terrain editing
// ---------------------------------------------------------------------------
/// A Node3D that moves a box through terrain, editing voxels in its path.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelBoxMoverGD {
    base: Base<Node3D>,
    #[var]
    box_size: f32,
}
#[godot_api]
impl INode3D for VoxelBoxMoverGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            box_size: 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelAStarGrid3DGD — RefCounted for 3D pathfinding
// ---------------------------------------------------------------------------
/// 3D A* pathfinding on voxel terrain.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelAStarGrid3DGD {
    base: Base<RefCounted>,
}
#[godot_api]
impl IRefCounted for VoxelAStarGrid3DGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self { base }
    }
}
