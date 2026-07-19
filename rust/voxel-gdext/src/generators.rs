//! Godot Resource bindings for voxel generators.
//!
//! Each `VoxelGenerator*` is a Godot `Resource` that wraps the corresponding
//! `voxel_core::generators::simple::*` type. When attached to a
//! [`VoxelTerrain`](crate::terrain::VoxelTerrain) via the `generator` property,
//! it produces voxel data on demand.

use godot::prelude::*;
use std::sync::Arc;

use voxel_core::generators::simple::{Flat, Waves};
use voxel_core::storage::SharedVoxelGenerator;

// ---------------------------------------------------------------------------
// VoxelGeneratorWaves
// ---------------------------------------------------------------------------

/// A simple SDF terrain generator that produces rolling waves along the X axis.
/// Wraps [`voxel_core::generators::simple::Waves`].
///
/// In GDScript: create a `VoxelGeneratorWaves` resource and assign it to a
/// `VoxelTerrain`'s `generator` property.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorWaves {
    base: Base<Resource>,
    /// Amplitude of the waves (in voxels).
    #[var]
    pub amplitude: f32,
    /// Frequency of the waves.
    #[var]
    pub frequency: f32,
    /// Period of the waves (alternative to frequency).
    #[var]
    pub period: f32,
}

#[godot_api]
impl IResource for VoxelGeneratorWaves {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            amplitude: 60.0,
            frequency: 0.02,
            period: 128.0,
        }
    }
}

#[godot_api]
impl VoxelGeneratorWaves {
    /// Construct the engine-agnostic generator from the current parameters.
    pub fn create_core_generator(&self) -> SharedVoxelGenerator {
        let mut waves = Waves::default();
        waves.set_pattern_size(voxel_core::math::Vector2f::new(self.period, self.period));
        waves.heightmap.height_range = self.amplitude;
        Arc::new(waves)
    }
}

// ---------------------------------------------------------------------------
// VoxelGeneratorFlat
// ---------------------------------------------------------------------------

/// A flat terrain generator that fills SDF as a horizontal plane at a given
/// height. Wraps [`voxel_core::generators::simple::Flat`].
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorFlat {
    base: Base<Resource>,
    /// Height of the flat surface (in voxels).
    #[var]
    pub height: i64,
}

#[godot_api]
impl IResource for VoxelGeneratorFlat {
    fn init(base: Base<Resource>) -> Self {
        Self { base, height: 0 }
    }
}

#[godot_api]
impl VoxelGeneratorFlat {
    /// Construct the engine-agnostic generator from the current parameters.
    pub fn create_core_generator(&self) -> SharedVoxelGenerator {
        let flat = Flat {
            height: self.height as f32,
            ..Flat::default()
        };
        Arc::new(flat)
    }
}
