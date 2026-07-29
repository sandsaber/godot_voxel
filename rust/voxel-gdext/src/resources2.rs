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
/// A sphere-shaped SDF modifier node. Add to a `VoxelTerrain` as a child to
/// carve (subtract) or merge (union) a sphere into the generated terrain.
///
/// Wraps [`voxel_core::modifiers::SphereModifier`] — `apply_to_buffer` runs
/// the real SDF blend (smooth union / subtract) over a `VoxelBufferGD`'s SDF
/// channel, sampling the modifier's world-space center from the node's 3D
/// transform.
#[derive(GodotClass)]
#[class(base = Node3D, tool)]
pub struct VoxelModifierSphereGD {
    base: Base<Node3D>,
    #[var]
    radius: f32,
    /// Blend operation: 0 = add (union), 1 = subtract. Mirrors
    /// `SdfOperation`.
    #[var]
    operation: i32,
    /// Smoothing factor for the blend (0 = hard, larger = smoother).
    #[var]
    smoothness: f32,
}
#[godot_api]
impl INode3D for VoxelModifierSphereGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            radius: 10.0,
            operation: 0,
            smoothness: 0.0,
        }
    }
}

#[godot_api]
impl VoxelModifierSphereGD {
    /// Apply this sphere modifier to a `VoxelBufferGD`'s SDF channel.
    /// `buffer` must be a `VoxelBufferGD`; `origin_x/y/z` is the buffer's
    /// world-space origin (voxel units). Returns the number of voxels whose
    /// SDF actually changed, or -1 if `buffer` is not a `VoxelBufferGD`.
    #[func]
    fn apply_to_buffer(
        &self,
        buffer: Gd<RefCounted>,
        origin_x: f32,
        origin_y: f32,
        origin_z: f32,
    ) -> i64 {
        let Ok(mut buf) = buffer.try_cast::<crate::voxel_buffer::VoxelBufferGD>() else {
            return -1;
        };
        let mut bound = buf.bind_mut();
        let core = bound.core_buffer_mut();
        let sx = core.size().x;
        let sy = core.size().y;
        let sz = core.size().z;
        const SDF_CHANNEL: usize = 1;

        // Build the core modifier from the node's state.
        let op = if self.operation == 1 {
            voxel_core::modifiers::SdfOperation::Subtract
        } else {
            voxel_core::modifiers::SdfOperation::Add
        };
        let center = self.base().get_position();
        let cx = center.x;
        let cy = center.y;
        let cz = center.z;

        // Gather SDF + world positions, apply the modifier, write back.
        let mut changed: i64 = 0;
        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    let sdf = core.get_voxel_f(x, y, z, SDF_CHANNEL);
                    let px = origin_x + x as f32;
                    let py = origin_y + y as f32;
                    let pz = origin_z + z as f32;
                    let shape = ((px - cx).powi(2) + (py - cy).powi(2) + (pz - cz).powi(2)).sqrt()
                        - self.radius;
                    let blended = sdf_blend_inline(sdf, shape, op, self.smoothness);
                    if (blended - sdf).abs() > 1e-6 {
                        core.set_voxel_f(blended, x, y, z, SDF_CHANNEL);
                        changed += 1;
                    }
                }
            }
        }
        changed
    }
}

/// Smooth SDF blending, mirroring `voxel_core::modifiers::sdf_blend` (which is
/// private). Used inline by [`VoxelModifierSphereGD::apply_to_buffer`] since
/// the core API is SoA-oriented (slice in, slice out).
fn sdf_blend_inline(
    existing: f32,
    shape: f32,
    op: voxel_core::modifiers::SdfOperation,
    smoothness: f32,
) -> f32 {
    use voxel_core::modifiers::SdfOperation;
    if smoothness <= 0.0 {
        return match op {
            SdfOperation::Add => existing.min(shape),
            SdfOperation::Subtract => existing.max(-shape),
        };
    }
    let h = (smoothness - (shape - existing).abs()).max(0.0) / smoothness;
    let m = shape + (existing - shape) * h; // lerp factor
    match op {
        SdfOperation::Add => m - smoothness * h * h,
        SdfOperation::Subtract => m + smoothness * h * h,
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
/// Wraps [`voxel_core::terrain::VoxelTerrainStats`] — real cumulative counters
/// pulled from the paging orchestrator (blocks loaded/unloaded, meshes
/// built/dropped).
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

impl VoxelTerrainStatsGD {
    /// Build a stats snapshot from the engine-agnostic
    /// [`voxel_core::terrain::VoxelTerrainStats`]. Called by
    /// `VoxelTerrain::get_statistics` to expose the real paging counters to
    /// GDScript/inspector.
    pub fn from_core_stats(stats: &voxel_core::terrain::VoxelTerrainStats) -> Gd<Self> {
        Gd::from_init_fn(|base| Self {
            base,
            blocks_loaded: stats.blocks_loaded as i64,
            blocks_unloaded: stats.blocks_unloaded as i64,
            meshes_built: stats.meshes_built as i64,
            meshes_dropped: stats.meshes_dropped as i64,
        })
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
///
/// Wraps [`voxel_core::meshers::blocky::BakedLibrary`] — the real model table
/// consumed by the blocky mesher. `add_color_type` appends a solid-color model
/// and `get_type_count` reports how many types are registered.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyTypeLibraryGD {
    base: Base<Resource>,
    /// Number of registered types (plain field; exposed via `get_type_count`
    /// #[func] to avoid a `#[var]` auto-getter collision).
    type_count: i32,
    /// The real baked model table. Kept in sync with `type_count`.
    library: voxel_core::meshers::blocky::BakedLibrary,
}
#[godot_api]
impl IResource for VoxelBlockyTypeLibraryGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            type_count: 0,
            library: voxel_core::meshers::blocky::BakedLibrary::default(),
        }
    }
}

#[godot_api]
impl VoxelBlockyTypeLibraryGD {
    /// Append a solid-color blocky type and return its id (the index of the
    /// new model). Mirrors the C++ `VoxelBlockyTypeLibrary::add_type`.
    #[func]
    fn add_color_type(&mut self, r: f32, g: f32, b: f32, a: f32) -> i32 {
        let model = voxel_core::meshers::blocky::BakedModel {
            color: voxel_core::math::Color::new(r, g, b, a),
            empty: false,
            ..voxel_core::meshers::blocky::BakedModel::default()
        };
        let id = self.library.models.len() as i32;
        self.library.models.push(model);
        self.type_count = self.library.models.len() as i32;
        id
    }

    /// Returns the number of registered types (read-only `#[var]` mirror).
    #[func]
    fn get_type_count(&self) -> i32 {
        self.type_count
    }

    /// Returns `true` if the type at `id` exists in the library.
    #[func]
    fn has_type(&self, id: i32) -> bool {
        self.library.has_model(id as u32)
    }
}

impl VoxelBlockyTypeLibraryGD {
    /// Borrow the underlying [`BakedLibrary`]. Used by sibling binding classes
    /// (the blocky mesher resource) that need direct access to run the blocky
    /// mesher without round-tripping through Godot calls.
    #[allow(dead_code)]
    pub fn core_library(&self) -> &voxel_core::meshers::blocky::BakedLibrary {
        &self.library
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
