//! Godot Resource bindings for voxel generators.
//!
//! Each `VoxelGenerator*` is a Godot `Resource` that wraps the corresponding
//! `voxel_core::generators::simple::*` type. When attached to a
//! [`VoxelTerrain`](crate::terrain::VoxelTerrain) via the `generator` property,
//! it produces voxel data on demand.

use godot::prelude::*;
use std::sync::Arc;

use voxel_core::generators::base::HeightmapParams;
use voxel_core::generators::simple::{
    Flat, HeightmapNoise, Image, ImageWrapMode, Noise, NoiseConfig, Waves,
};
use voxel_core::storage::voxel_buffer::channel_id_from_index;
use voxel_core::storage::{ChannelId, SharedVoxelGenerator};

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

    /// Sample the terrain height at world `(x, z)`. Returns the height value
    /// (noise remapped to `[0, height_range]`). Deterministic for a fixed
    /// seed/frequency.
    #[func]
    fn sample_height(&self, x: f32, z: f32) -> f32 {
        let config = NoiseConfig {
            seed: Some(self.seed as i32),
            frequency: Some(self.frequency),
            ..NoiseConfig::default()
        };
        let noise = config.build();
        let n = noise.get_noise_2d(x, z);
        // Match HeightmapNoise's default (no curve): 0.5 + 0.5*noise → height_range.
        (0.5 + 0.5 * n) * self.height_range
    }
}

// ---------------------------------------------------------------------------
// VoxelGeneratorImage
// ---------------------------------------------------------------------------

/// A heightmap terrain generator driven by an image. Pixel luminance becomes
/// terrain height: `height = height_start + luminance * height_range`.
/// Wraps [`voxel_core::generators::simple::Image`].
///
/// With `channel = 1` (SDF) this produces smooth transvoxel terrain; with
/// `channel = 0` (Type) it fills blocky voxels, which can drive the cubes
/// mesher (Minecraft-style terrain from an image).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGeneratorImage {
    base: Base<Resource>,
    /// Vertical extent of the terrain; pixel values `0..1` scale by this.
    #[var]
    pub height_range: f32,
    /// World Y that a black pixel (0) maps to.
    #[var]
    pub height_start: f32,
    /// Output channel: `0` = Type (blocky), `1` = Sdf (smooth).
    #[var]
    pub channel: i32,
    /// Tile the image horizontally instead of clamping at its edges.
    #[var]
    pub repeat: bool,
    /// Normalized heights (`0..1`), row-major `x + z * width`.
    values: Vec<f32>,
    /// Image size: `[width, height]`.
    size: [i32; 2],
}

#[godot_api]
impl IResource for VoxelGeneratorImage {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            height_range: 100.0,
            height_start: -50.0,
            channel: ChannelId::Sdf.index() as i32,
            repeat: false,
            values: Vec::new(),
            size: [0, 0],
        }
    }
}

#[godot_api]
impl VoxelGeneratorImage {
    /// Load heights from a Godot `Image`: each pixel's luminance becomes the
    /// normalized height at that `(x, z)`. Replaces any previously loaded
    /// image.
    #[func]
    fn set_image(&mut self, image: Gd<godot::classes::Image>) -> bool {
        let width = image.get_width();
        let height = image.get_height();
        if width <= 0 || height <= 0 {
            return false;
        }
        let mut values = Vec::with_capacity((width * height) as usize);
        for z in 0..height {
            for x in 0..width {
                let c = image.get_pixel(x, z);
                // Rec. 709 luminance.
                values.push(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b);
            }
        }
        self.values = values;
        self.size = [width, height];
        true
    }

    /// Load heights from raw bytes (`0..255` → `0..1`), row-major.
    /// Returns `false` if `data.len() != width * height`.
    #[func]
    fn set_heights(&mut self, data: PackedByteArray, width: i32, height: i32) -> bool {
        if width <= 0 || height <= 0 || data.len() != (width * height) as usize {
            return false;
        }
        self.values = data.as_slice().iter().map(|&b| b as f32 / 255.0).collect();
        self.size = [width, height];
        true
    }

    /// Whether an image/heightmap is loaded.
    #[func]
    fn has_image(&self) -> bool {
        !self.values.is_empty()
    }

    /// Construct the engine-agnostic generator from the current parameters.
    pub fn create_core_generator(&self) -> SharedVoxelGenerator {
        let mut gen = Image::default();
        gen.set_image(self.values.clone(), self.size[0], self.size[1]);
        gen.wrap = if self.repeat {
            ImageWrapMode::Repeat
        } else {
            ImageWrapMode::Clamp
        };
        gen.heightmap.height_start = self.height_start;
        gen.heightmap.height_range = self.height_range;
        gen.heightmap.channel =
            channel_id_from_index(self.channel.max(0) as usize).unwrap_or(ChannelId::Sdf);
        Arc::new(gen)
    }
}
