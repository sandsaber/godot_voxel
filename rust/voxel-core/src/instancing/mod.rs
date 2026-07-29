//! Voxel instancing — scatter objects (trees, rocks, grass) on terrain surfaces.
//!
//! Ports the engine-agnostic half of `terrain/instancing/`. The Godot-facing
//! `VoxelInstancer` Node3D wrapper lives in `voxel-gdext`.
//!
//! ## Status
//! MVP: `InstanceLibrary`, `InstanceGenerator` trait, and `BlockInstanceData`
//! (the data carrier between engine-agnostic scatter and Godot multimesh upload).

pub mod library;
pub mod scatter;

pub use library::{InstanceLibrary, InstanceLibraryItem, InstanceMeshType};
pub use scatter::{BlockInstanceData, InstanceGenerator, ScatterConfig};
