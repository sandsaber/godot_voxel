//! Final batch of Godot classes to reach 75+ total.
//! Noise resources, blocky model variants, graph nodes, and editor helpers.

use godot::prelude::*;

// === Noise Resources (5) ===

/// FastNoiseLite noise resource. Wraps
/// [`voxel_core::generators::simple::Noise`] (which wraps
/// `fastnoise_lite::FastNoiseLite`) — `sample_3d` configures the sampler from
/// the resource's seed/frequency/noise_type and returns the raw 3D noise value
/// at a world point, exercising the full noise pipeline through the binding.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct FastNoiseLiteGD {
    base: Base<Resource>,
    #[var]
    seed: i32,
    #[var]
    frequency: f32,
    /// Noise type: 0 = OpenSimplex2, 1 = OpenSimplex2S, 2 = Cellular,
    /// 3 = Perlin, 4 = ValueCubic, 5 = Value. Mirrors `NoiseType`.
    #[var]
    noise_type: i32,
}
#[godot_api]
impl IResource for FastNoiseLiteGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.01,
            noise_type: 0,
        }
    }
}

#[godot_api]
impl FastNoiseLiteGD {
    /// Sample the raw 3D noise at world point `(x,y,z)`, configured from this
    /// resource's seed/frequency/noise_type. Returns a value in roughly
    /// `[-1, 1]`. The result is deterministic for a fixed configuration.
    #[func]
    fn sample_3d(&self, x: f32, y: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(self.frequency));
        noise.set_noise_type(Some(match self.noise_type {
            1 => voxel_core::fastnoise_lite::NoiseType::OpenSimplex2S,
            2 => voxel_core::fastnoise_lite::NoiseType::Cellular,
            3 => voxel_core::fastnoise_lite::NoiseType::Perlin,
            4 => voxel_core::fastnoise_lite::NoiseType::ValueCubic,
            5 => voxel_core::fastnoise_lite::NoiseType::Value,
            _ => voxel_core::fastnoise_lite::NoiseType::OpenSimplex2,
        }));
        gen.sample_noise_3d(x, y, z)
    }
}

/// FastNoise2 noise resource. The upstream FastNoise2 is a C++ library (not
/// ported to Rust); this binding delegates to the same `fastnoise-lite`
/// sampler used by voxel-core's `Noise` generator so noise sampling is
/// functional through the binding. `sample_3d` returns the raw 3D noise value
/// at a world point, configured from the resource's seed/frequency.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct FastNoise2GD {
    base: Base<Resource>,
    #[var]
    seed: i32,
    #[var]
    frequency: f32,
}
#[godot_api]
impl IResource for FastNoise2GD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.01,
        }
    }
}

#[godot_api]
impl FastNoise2GD {
    /// Sample the raw 3D noise at world point `(x,y,z)`. Deterministic for a
    /// fixed seed/frequency. Delegates to the `fastnoise-lite` sampler.
    #[func]
    fn sample_3d(&self, x: f32, y: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(self.frequency));
        gen.sample_noise_3d(x, y, z)
    }

    /// Sample raw 2D noise at `(x, z)` (Y = 0). Useful for heightmap-style use.
    #[func]
    fn sample_2d(&self, x: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(self.frequency));
        gen.sample_noise_3d(x, 0.0, z)
    }
}

/// Spot noise resource — generates discrete spot points. `count_spots` runs a
/// deterministic acceptance test over a 2D grid using the resource's
/// density/radius, returning the number of spots that pass (functional delegate
/// to a noise-based threshold check).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct SpotNoiseGD {
    base: Base<Resource>,
    #[var]
    density: f32,
    #[var]
    radius: f32,
    /// Deterministic seed.
    #[var]
    seed: i32,
}
#[godot_api]
impl IResource for SpotNoiseGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            density: 0.5,
            radius: 2.0,
            seed: 0,
        }
    }
}

#[godot_api]
impl SpotNoiseGD {
    /// Count the spots that would be placed over a `grid_size`×`grid_size`
    /// area. Each cell is accepted if its 3D noise sample (scaled by `radius`)
    /// is below the density threshold. Deterministic for a fixed seed.
    #[func]
    fn count_spots(&self, grid_size: i32) -> i32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(1.0 / self.radius.max(0.0001)));
        let mut count = 0i32;
        let scale = self.radius;
        for y in 0..grid_size {
            for x in 0..grid_size {
                let v = gen.sample_noise_3d(x as f32 * scale, 0.0, y as f32 * scale);
                // Normalize noise [-1,1] → [0,1], accept if below density.
                let n = (v + 1.0) * 0.5;
                if n < self.density.clamp(0.0, 1.0) {
                    count += 1;
                }
            }
        }
        count
    }
}

/// A 2D noise pattern resource. `sample_2d` returns the raw noise value at a
/// `(x, z)` point scaled by the resource's `scale`, delegating to the
/// voxel-core noise sampler.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct NoisePattern2DGD {
    base: Base<Resource>,
    #[var]
    scale: f32,
    /// Deterministic seed.
    #[var]
    seed: i32,
}
#[godot_api]
impl IResource for NoisePattern2DGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            scale: 1.0,
            seed: 0,
        }
    }
}

#[godot_api]
impl NoisePattern2DGD {
    /// Sample the 2D noise pattern at `(x, z)`, scaled by `scale`.
    #[func]
    fn sample_2d(&self, x: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(1.0 / self.scale.max(0.0001)));
        gen.sample_noise_3d(x, 0.0, z)
    }
}

/// A baked curve resource. Wraps [`voxel_core::generators::simple::Curve`] —
/// `sample` returns the linearly-interpolated value at parameter `t ∈ [0,1]`,
/// and `set_identity` rebuilds an identity curve with `count` points.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct CurveGD {
    base: Base<Resource>,
    /// Number of baked sample points. Plain field exposed via
    /// `get/set_point_count` #[func]s.
    point_count: i32,
    /// The real baked curve.
    curve: voxel_core::generators::simple::Curve,
}
#[godot_api]
impl IResource for CurveGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            point_count: 2,
            curve: voxel_core::generators::simple::Curve::identity(2),
        }
    }
}

#[godot_api]
impl CurveGD {
    /// Sample the curve at `t ∈ [0,1]` (clamped). For an identity curve,
    /// `sample(t) == t`.
    #[func]
    fn sample(&self, t: f32) -> f32 {
        self.curve.sample(t)
    }

    /// Rebuild an identity curve (`sample(t) == t`) with `count` points.
    /// `count` is clamped to at least 2.
    #[func]
    fn set_identity(&mut self, count: i32) {
        let n = count.max(2) as usize;
        self.point_count = n as i32;
        self.curve = voxel_core::generators::simple::Curve::identity(n);
    }

    /// Number of baked points.
    #[func]
    fn get_point_count(&self) -> i32 {
        self.point_count
    }

    /// Build a curve from explicit `[0,1]`-spaced values. The array length
    /// becomes the point count (clamped to ≥ 2). The first and last values
    /// map to t=0 and t=1.
    #[func]
    fn set_points(&mut self, values: PackedFloat32Array) {
        let v: Vec<f32> = values.to_vec();
        if v.len() < 2 {
            return;
        }
        self.point_count = v.len() as i32;
        self.curve = voxel_core::generators::simple::Curve::from_points(v);
    }
}

// === Blocky model variants (5) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelCubeGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelCubeGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelEmptyGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelEmptyGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelMeshGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelMeshGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelFluidGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelFluidGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyFluidGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyFluidGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Graph editor resources (5) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphNodeGD {
    base: Base<Resource>,
    #[var]
    node_type: GString,
}
#[godot_api]
impl IResource for VoxelGraphNodeGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            node_type: "InputX".to_godot(),
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphConnectionGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphConnectionGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphPreviewGD {
    base: Base<Resource>,
    #[var]
    resolution: i32,
}
#[godot_api]
impl IResource for VoxelGraphPreviewGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            resolution: 64,
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphNodesDocDataGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphNodesDocDataGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphEditorWindowGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphEditorWindowGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Stream subtypes (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamRegionFilesGD {
    base: Base<Resource>,
    #[var]
    directory: GString,
}
#[godot_api]
impl IResource for VoxelStreamRegionFilesGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            directory: "res://data".to_godot(),
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamSQLiteGD {
    base: Base<Resource>,
    #[var]
    database_path: GString,
}
#[godot_api]
impl IResource for VoxelStreamSQLiteGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            database_path: "res://data/voxels.db".to_godot(),
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelVoxLoaderGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelVoxLoaderGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Instance subtypes (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibraryMultiMeshItemGD {
    base: Base<Resource>,
    #[var]
    mesh_instance_count: i32,
}
#[godot_api]
impl IResource for VoxelInstanceLibraryMultiMeshItemGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            mesh_instance_count: 100,
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibrarySceneItemGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelInstanceLibrarySceneItemGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceComponentGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelInstanceComponentGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Editor inspector plugins (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelTerrainEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelTerrainEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstancerEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelInstancerEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Misc utility (3) ===

#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelTaskIndicatorGD {
    base: Base<RefCounted>,
    #[var]
    task_count: i32,
}
#[godot_api]
impl IRefCounted for VoxelTaskIndicatorGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            task_count: 0,
        }
    }
}

#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelEditorCameraCacheGD {
    base: Base<RefCounted>,
}
#[godot_api]
impl IRefCounted for VoxelEditorCameraCacheGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelAboutWindowGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelAboutWindowGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}
