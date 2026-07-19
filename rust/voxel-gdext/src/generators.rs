//! Godot Resource bindings for voxel generators.
//!
//! Each `VoxelGenerator*` is a Godot `Resource` that wraps the corresponding
//! `voxel_core::generators::simple::*` type. When attached to a
//! [`VoxelTerrain`](crate::terrain::VoxelTerrain) via the `generator` property,
//! it produces voxel data on demand.

use godot::prelude::*;
use std::sync::Arc;

use voxel_core::generators::base::HeightmapParams;
use voxel_core::generators::simple::{Flat, HeightmapNoise, Noise, NoiseConfig, Waves};
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

// ---------------------------------------------------------------------------
// VoxelGeneratorNoise
// ---------------------------------------------------------------------------

/// A 3D noise terrain generator. Produces caves / overhangs via 3D FastNoiseLite.
/// Wraps [`voxel_core::generators::simple::Noise`].
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorNoise {
    base: Base<Resource>,
    /// Random seed for the noise.
    #[var]
    pub seed: i64,
    /// Noise frequency (higher = more detail).
    #[var]
    pub frequency: f32,
    /// Bottom of the noise slab (world Y).
    #[var]
    pub height_start: f32,
    /// Vertical extent of the slab.
    #[var]
    pub height_range: f32,
}

#[godot_api]
impl IResource for VoxelGeneratorNoise {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.05,
            height_start: -100.0,
            height_range: 200.0,
        }
    }
}

#[godot_api]
impl VoxelGeneratorNoise {
    pub fn create_core_generator(&self) -> SharedVoxelGenerator {
        // Use NoiseConfig.build() to avoid direct fastnoise_lite dependency.
        let config = NoiseConfig {
            seed: Some(self.seed as i32),
            frequency: Some(self.frequency),
            ..NoiseConfig::default()
        };
        let noise = Noise {
            noise: config.build(),
            height_start: self.height_start,
            height_range: self.height_range,
            ..Noise::default()
        };
        Arc::new(noise)
    }
}

// ---------------------------------------------------------------------------
// VoxelGeneratorHeightmap
// ---------------------------------------------------------------------------

/// A heightmap terrain generator driven by 2D noise. Produces rolling hills
/// with controllable seed, frequency, and height range.
/// Wraps [`voxel_core::generators::simple::HeightmapNoise`].
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorHeightmap {
    base: Base<Resource>,
    /// Random seed.
    #[var]
    pub seed: i64,
    /// Noise frequency.
    #[var]
    pub frequency: f32,
    /// Height range of the terrain (amplitude).
    #[var]
    pub height_range: f32,
}

#[godot_api]
impl IResource for VoxelGeneratorHeightmap {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.02,
            height_range: 100.0,
        }
    }
}

#[godot_api]
impl VoxelGeneratorHeightmap {
    pub fn create_core_generator(&self) -> SharedVoxelGenerator {
        let config = NoiseConfig {
            seed: Some(self.seed as i32),
            frequency: Some(self.frequency),
            ..NoiseConfig::default()
        };
        let hm = HeightmapNoise {
            noise_config: config,
            curve: None,
            heightmap: HeightmapParams {
                height_range: self.height_range,
                ..Default::default()
            },
        };
        Arc::new(hm)
    }
}
