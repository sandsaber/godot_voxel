//! Terrain — paging orchestrators that drive [`crate::storage::VoxelData`] +
//! [`crate::meshers::MeshBlockTask`] based on viewer positions.
//!
//! Ports the engine-agnostic core of `terrain/fixed_lod/voxel_terrain.cpp`
//! (single-LOD paging) and (later) the multi-LOD equivalent under
//! `terrain/variable_lod/`. Godot `Node3D` / `RenderingServer` glue lives in
//! the `voxel-gdext` crate.

pub mod lod_octree;
pub mod voxel_terrain_core;

pub use lod_octree::{LodOctree, OctreeNodeData, OctreeUpdateActions};
pub use voxel_terrain_core::{
    MeshBlockEntry, PairedViewer, ViewerId, ViewerState, VoxelTerrainCore, VoxelTerrainEvent,
    VoxelTerrainStats,
};
