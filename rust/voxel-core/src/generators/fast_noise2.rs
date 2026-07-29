//! FastNoise2 — a noise generator compatible with the FastNoise2 C++ API.
//!
//! This is a pure-Rust implementation that provides the same noise generation
//! interface as the C++ FastNoise2 library used by the voxel engine. It wraps
//! `fastnoise-lite` (already a dependency) with an API matching
//! `FastNoise2::get_noise_2d_single` / `get_noise_3d_single`.
//!
//! The C++ FastNoise2 supports node-tree-based noise generation (EncodedNodeTree).
//! This Rust port provides a simplified API that covers the basic noise types
//! and fractal settings, which is sufficient for the voxel engine's use cases.

use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};

/// Noise type enum matching C++ FastNoise2::Type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FastNoise2Type {
    /// Simple noise from a single FastNoiseLite instance.
    #[default]
    Simple,
    /// Encoded node tree (C++ specific — falls back to Simple in Rust).
    EncodedNodeTree,
}

/// A FastNoise2-compatible noise generator. Wraps FastNoiseLite with the
/// same API surface the voxel engine expects.
pub struct FastNoise2 {
    noise: FastNoiseLite,
    noise_type: FastNoise2Type,
    simd_level: u32,
}

impl Default for FastNoise2 {
    fn default() -> Self {
        Self::new()
    }
}

impl FastNoise2 {
    /// Create a new FastNoise2 with default settings.
    pub fn new() -> Self {
        let mut noise = FastNoiseLite::new();
        noise.set_noise_type(Some(NoiseType::OpenSimplex2));
        noise.set_frequency(Some(0.02));
        noise.set_fractal_type(Some(FractalType::FBm));
        noise.set_fractal_octaves(Some(5));
        Self {
            noise,
            noise_type: FastNoise2Type::Simple,
            simd_level: 0, // Pure Rust — no SIMD level
        }
    }

    /// Get 2D noise at a single point. Matches `FastNoise2::get_noise_2d_single`.
    pub fn get_noise_2d_single(&self, x: f32, y: f32) -> f32 {
        self.noise.get_noise_2d(x, y)
    }

    /// Get 3D noise at a single point. Matches `FastNoise2::get_noise_3d_single`.
    pub fn get_noise_3d_single(&self, x: f32, y: f32, z: f32) -> f32 {
        self.noise.get_noise_3d(x, y, z)
    }

    /// Set the seed.
    pub fn set_seed(&mut self, seed: i32) {
        self.noise.set_seed(Some(seed));
    }

    /// Get the current seed.
    pub fn get_seed(&self) -> i32 {
        self.noise.seed
    }

    /// Set frequency.
    pub fn set_frequency(&mut self, freq: f32) {
        self.noise.set_frequency(Some(freq));
    }

    /// Set noise type (OpenSimplex2, Perlin, etc.).
    pub fn set_noise_type_lite(&mut self, noise_type: NoiseType) {
        self.noise.set_noise_type(Some(noise_type));
    }

    /// Set fractal type.
    pub fn set_fractal_type(&mut self, fractal_type: FractalType) {
        self.noise.set_fractal_type(Some(fractal_type));
    }

    /// Set fractal octaves.
    pub fn set_fractal_octaves(&mut self, octaves: i32) {
        self.noise.set_fractal_octaves(Some(octaves));
    }

    /// Set the FastNoise2 noise type (Simple or EncodedNodeTree).
    pub fn set_noise_type(&mut self, noise_type: FastNoise2Type) {
        self.noise_type = noise_type;
    }

    /// Get the FastNoise2 noise type.
    pub fn get_noise_type(&self) -> FastNoise2Type {
        self.noise_type
    }

    /// Update the generator (for EncodedNodeTree type — no-op in Rust).
    pub fn update_generator(&self) {
        // EncodedNodeTree falls back to Simple in Rust; no crash.
    }

    /// Get the SIMD level name (always "Scalar" in pure Rust).
    pub fn get_simd_level(&self) -> u32 {
        self.simd_level
    }

    /// Get the SIMD level name as a string.
    pub fn get_simd_level_name() -> &'static str {
        "Scalar (Rust)"
    }

    /// Generate a 2D image of noise values into a flat buffer.
    /// `width` × `height` values, each in [-1, 1].
    pub fn generate_image_2d(&self, width: u32, height: u32) -> Vec<f32> {
        let mut result = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                result.push(self.get_noise_2d_single(x as f32, y as f32));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_2d_noise_not_nan() {
        let noise = FastNoise2::new();
        let v = noise.get_noise_2d_single(42.0, 666.0);
        assert!(v.is_finite(), "noise should be finite: {v}");
    }

    #[test]
    fn basic_3d_noise_not_nan() {
        let noise = FastNoise2::new();
        let v = noise.get_noise_3d_single(1.0, 2.0, 3.0);
        assert!(v.is_finite(), "3D noise should be finite: {v}");
    }

    #[test]
    fn noise_in_valid_range() {
        let noise = FastNoise2::new();
        for x in 0..20 {
            for y in 0..20 {
                let v = noise.get_noise_2d_single(x as f32, y as f32);
                assert!(
                    v >= -1.5 && v <= 1.5,
                    "noise out of range at ({x},{y}): {v}"
                );
            }
        }
    }

    #[test]
    fn different_seeds_produce_different_noise() {
        let mut a = FastNoise2::new();
        a.set_seed(1);
        let mut b = FastNoise2::new();
        b.set_seed(2);
        let va = a.get_noise_2d_single(5.0, 5.0);
        let vb = b.get_noise_2d_single(5.0, 5.0);
        assert!(
            (va - vb).abs() > 1e-6,
            "different seeds should differ: {va} vs {vb}"
        );
    }

    #[test]
    fn deterministic_same_seed_same_result() {
        let mut a = FastNoise2::new();
        a.set_seed(42);
        let mut b = FastNoise2::new();
        b.set_seed(42);
        assert!((a.get_noise_2d_single(7.0, 3.0) - b.get_noise_2d_single(7.0, 3.0)).abs() < 1e-7);
    }

    #[test]
    fn encoded_node_tree_no_crash() {
        let mut noise = FastNoise2::new();
        noise.set_noise_type(FastNoise2Type::EncodedNodeTree);
        noise.update_generator(); // should not crash
        let v = noise.get_noise_2d_single(1.0, 1.0);
        assert!(v.is_finite());
    }

    #[test]
    fn simd_level_name() {
        assert_eq!(FastNoise2::get_simd_level_name(), "Scalar (Rust)");
    }

    #[test]
    fn generate_image_2d_correct_size() {
        let noise = FastNoise2::new();
        let img = noise.generate_image_2d(16, 8);
        assert_eq!(img.len(), 128);
        for &v in &img {
            assert!(v.is_finite());
        }
    }

    #[test]
    fn set_frequency_changes_output() {
        let mut low = FastNoise2::new();
        low.set_frequency(0.01);
        let mut high = FastNoise2::new();
        high.set_frequency(0.5);
        let vl = low.get_noise_2d_single(5.0, 5.0);
        let vh = high.get_noise_2d_single(5.0, 5.0);
        assert!(
            (vl - vh).abs() > 1e-6,
            "different frequency should differ: {vl} vs {vh}"
        );
    }
}
