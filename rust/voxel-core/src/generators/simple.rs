//! `generators::simple` — math-pure terrain generators.
//!
//! Ported from `generators/simple/{voxel_generator_waves,voxel_generator_flat}.
//! {h,cpp}`. Both are heightmap-style generators: [`Waves`] uses a sinusoid,
//! [`Flat`] uses a constant. Neither pulls in a noise library — `Noise` /
//! `HeightmapNoise` land separately with `fastnoise-lite`.
//!
//! The C++ versions inherit `VoxelGenerator` (a Godot `Resource` with an
//! `RWLock` around their parameter struct) and `VoxelGeneratorHeightmap`. Here
//! each generator owns its parameters by value and implements [`VoxelGenerator`];
//! parameter mutation is single-threaded by Rust's `&mut` borrow rules.

use crate::generators::base::{
    generate_heightmap, GenResult, HeightmapParams, VoxelGenerator, VoxelQueryData,
};
use crate::math::funcs;
use crate::math::{Vector2f, Vector3i};
use crate::storage::voxel_buffer::ChannelId;

// ===========================================================================
// Waves
// ===========================================================================

/// Sinusoidal heightmap generator. Ported from `VoxelGeneratorWaves`.
///
/// Produces terrain height `0.5 + 0.25 * (cos((x+ox)*fx) + sin((z+oz)*fz))`
/// where `f = pi / pattern_size`, before the heightmap range remap.
#[derive(Debug, Clone, PartialEq)]
pub struct Waves {
    /// Period of the wave pattern along each axis. Clamped to `>= 0` on set.
    pub pattern_size: Vector2f,
    /// Phase offset (in voxels) of the pattern along each axis.
    pub pattern_offset: Vector2f,
    /// Shared heightmap parameters (channel, range, iso_scale, …).
    pub heightmap: HeightmapParams,
}

impl Default for Waves {
    fn default() -> Self {
        // C++ ctor: pattern_size (30, 30), height_range 30.
        Self {
            pattern_size: Vector2f::new(30.0, 30.0),
            pattern_offset: Vector2f::new(0.0, 0.0),
            heightmap: HeightmapParams {
                height_start: 0.0,
                height_range: 30.0,
                ..Default::default()
            },
        }
    }
}

impl Waves {
    /// Compute the raw (pre-range-remap) height at world `(x, z)`. Exposed for
    /// unit tests; matches the C++ lambda inside `generate_block`.
    pub fn height_at(&self, x: i32, z: i32) -> f32 {
        let fx = std::f32::consts::PI / self.pattern_size.x;
        let fz = std::f32::consts::PI / self.pattern_size.y;
        let ox = self.pattern_offset.x;
        let oz = self.pattern_offset.y;
        0.5 + 0.25 * (((x as f32 + ox) * fx).cos() + ((z as f32 + oz) * fz).sin())
    }

    /// `set_pattern_size` — clamps both components to `>= 0`.
    pub fn set_pattern_size(&mut self, size: Vector2f) {
        self.pattern_size = Vector2f::new(funcs::max(size.x, 0.0), funcs::max(size.y, 0.0));
    }

    /// `set_pattern_offset`.
    pub fn set_pattern_offset(&mut self, offset: Vector2f) {
        self.pattern_offset = offset;
    }
}

impl VoxelGenerator for Waves {
    fn generate_block(&mut self, input: VoxelQueryData<'_>) -> GenResult {
        let ps = self.pattern_size;
        let po = self.pattern_offset;
        let hp = self.heightmap;
        // Capture by value so the closure is `Fn` and borrows nothing.
        let height_fn = move |x: i32, z: i32| {
            let fx = std::f32::consts::PI / ps.x;
            let fz = std::f32::consts::PI / ps.y;
            0.5 + 0.25 * (((x as f32 + po.x) * fx).cos() + ((z as f32 + po.y) * fz).sin())
        };
        generate_heightmap(
            input.buffer,
            height_fn,
            &hp,
            input.origin_in_voxels,
            input.lod,
        )
    }
}

// ===========================================================================
// Flat
// ===========================================================================

/// A flat ground plane at a fixed height. Ported from `VoxelGeneratorFlat`.
///
/// Unlike [`Waves`], this generator does **not** go through the shared
/// heightmap helper: it has its own `generate_block` with an SDF and a blocky
/// path, plus two early-exit branches (block entirely above / below the
/// plane). The C++ version is the same — it overrides `generate_block`
/// directly rather than using `VoxelGeneratorHeightmap::generate`.
#[derive(Debug, Clone, PartialEq)]
pub struct Flat {
    /// Channel to write. Defaults to SDF.
    pub channel: ChannelId,
    /// Voxel id used when filling blocky terrain below `height`.
    pub voxel_type: u64,
    /// World-space Y of the ground plane.
    pub height: f32,
    /// SDF iso-surface scale (multiplies `y - height`).
    pub iso_scale: f32,
}

impl Default for Flat {
    fn default() -> Self {
        Self {
            channel: ChannelId::Sdf,
            voxel_type: 1,
            height: 0.0,
            iso_scale: 1.0,
        }
    }
}

impl Flat {
    pub fn set_channel(&mut self, channel: ChannelId) {
        self.channel = channel;
    }
    pub fn set_voxel_type(&mut self, t: u64) {
        self.voxel_type = t;
    }
    pub fn set_height(&mut self, h: f32) {
        self.height = h;
    }
    pub fn set_iso_scale(&mut self, s: f32) {
        self.iso_scale = s;
    }
}

impl VoxelGenerator for Flat {
    fn generate_block(&mut self, input: VoxelQueryData<'_>) -> GenResult {
        let channel = self.channel.index();
        let origin = input.origin_in_voxels;
        let bs = input.buffer.size();
        let use_sdf = self.channel == ChannelId::Sdf;
        let margin = 1i32 << input.lod;
        let lod = input.lod;

        // Block bottom above the highest ground → air.
        if (origin.y as f32) > self.height + margin as f32 {
            return GenResult::max_lod();
        }
        // Block top below the lowest ground → uniform fill.
        let block_top = origin.y + (bs.y << lod);
        if (block_top as f32) < self.height - margin as f32 {
            if use_sdf {
                // "Not consistent SDF but should work ok" — matches C++.
                input.buffer.clear_channel_f(channel, -100.0);
            } else {
                input.buffer.clear_channel(channel, self.voxel_type);
            }
            return GenResult::max_lod();
        }

        let stride = 1i32 << lod;
        if use_sdf {
            // Flat plane: height is constant, so the SDF depends only on Y.
            // (The C++ loop still tracks gx/gz for parity with the heightmap
            // generators, but they don't affect the output; we drop them.)
            for z in 0..bs.z {
                for x in 0..bs.x {
                    let mut gy = origin.y;
                    for y in 0..bs.y {
                        let sdf = self.iso_scale * (gy as f32 - self.height);
                        input.buffer.set_voxel_f(sdf, x, y, z, channel);
                        gy += stride;
                    }
                }
            }
        } else {
            // Blocky: fill [0, irh_voxels) across the whole block footprint.
            let rh_world = self.height - origin.y as f32;
            let irh_world = rh_world as i32;
            if irh_world > 0 {
                let irh_voxels = funcs::min(funcs::arithmetic_rshift(irh_world, lod), bs.y);
                input.buffer.fill_area(
                    self.voxel_type,
                    Vector3i::new(0, 0, 0),
                    Vector3i::new(bs.x, irh_voxels, bs.z),
                    channel,
                );
            }
        }

        GenResult::default()
    }

    fn used_channels_mask(&self) -> u32 {
        1 << self.channel.index()
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::math::{Vector2f, Vector2i, Vector3i};
    use crate::storage::voxel_buffer::{ChannelId, Compression};
    use crate::storage::VoxelBuffer;

    /// Build a fresh SDF-channel buffer of the given size.
    fn sdf_buffer(size: Vector3i) -> VoxelBuffer {
        // SDF starts uniform; the generators decompress channels as they write.
        VoxelBuffer::with_size(size)
    }

    // ---- Waves: height function ------------------------------------------

    #[test]
    fn waves_height_at_zero_offset_is_half_plus_quarter_cos_sin() {
        let w = Waves {
            pattern_size: Vector2f::new(30.0, 30.0),
            pattern_offset: Vector2f::new(0.0, 0.0),
            ..Default::default()
        };
        // At (0, 0): cos(0) + sin(0) = 1 + 0 = 1 → 0.5 + 0.25 = 0.75.
        assert!((w.height_at(0, 0) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn waves_height_is_bounded_between_0_and_1() {
        let w = Waves::default();
        // The sinusoid's range is 0.5 ± 0.5, so any integer (x, z) lands inside.
        for x in -100..100 {
            for z in -100..100 {
                let h = w.height_at(x, z);
                assert!(
                    (-1e-5..=1.0 + 1e-5).contains(&h),
                    "height {h} out of [0,1] at ({x},{z})"
                );
            }
        }
    }

    #[test]
    fn waves_set_pattern_size_clamps_negative_to_zero() {
        let mut w = Waves::default();
        w.set_pattern_size(Vector2f::new(-5.0, -10.0));
        assert_eq!(w.pattern_size, Vector2f::new(0.0, 0.0));
    }

    #[test]
    fn waves_pattern_offset_shifts_the_phase() {
        let mut w = Waves::default();
        w.pattern_size = Vector2f::new(30.0, 30.0);
        let h0 = w.height_at(0, 0);
        // Shifting by exactly the pattern period (2*pi * size, but our freq is
        // pi/size so the period is 2*size) must return the same height.
        w.pattern_offset = Vector2f::new(60.0, 60.0);
        let h_shifted = w.height_at(0, 0);
        assert!(
            (h0 - h_shifted).abs() < 1e-4,
            "period shift mismatch: {h0} vs {h_shifted}"
        );
    }

    // ---- Flat: SDF path --------------------------------------------------

    #[test]
    fn flat_sdf_gradient_grows_with_y_and_crosses_zero_at_height() {
        let mut gen = Flat::default();
        gen.height = 4.0;
        let mut buf = sdf_buffer(Vector3i::new(2, 8, 2));
        // Block spans world Y 0..8 with the plane at y=4.
        gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: Vector3i::new(0, 0, 0),
            lod: 0,
        });
        // Below the plane: negative SDF (solid); at the plane: ~0; above: positive.
        assert!(buf.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()) < 0.0);
        assert!(buf.get_voxel_f(0, 7, 0, ChannelId::Sdf.index()) > 0.0);
        assert!(buf.get_voxel_f(0, 4, 0, ChannelId::Sdf.index()).abs() < 1.0);
    }

    #[test]
    fn flat_sdf_uses_iso_scale() {
        let mut gen = Flat::default();
        gen.height = 0.0;
        gen.iso_scale = 0.1;
        let mut buf = sdf_buffer(Vector3i::new(1, 4, 1));
        // Use 32-bit depth so the SDF round-trips without 16-bit snorm
        // quantization (storage quantizes Bit16 SDFs to [-1,1] via snorm).
        buf.set_channel_depth(ChannelId::Sdf.index(), crate::storage::ChannelDepth::Bit32);
        gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: Vector3i::new(0, 0, 0),
            lod: 0,
        });
        // At y=1 with iso_scale=0.1: sdf = 0.1 * (1 - 0) = 0.1.
        let v = buf.get_voxel_f(0, 1, 0, ChannelId::Sdf.index());
        assert!((v - 0.1).abs() < 1e-5, "sdf at y=1: {v}");
    }

    // ---- Flat: blocky path ----------------------------------------------

    #[test]
    fn flat_blocky_fills_below_height() {
        let mut gen = Flat::default();
        gen.channel = ChannelId::Type;
        gen.voxel_type = 7;
        gen.height = 3.0;
        let mut buf = VoxelBuffer::with_size(Vector3i::new(2, 8, 2));
        gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: Vector3i::new(0, 0, 0),
            lod: 0,
        });
        // y < 3 should be solid (7), y >= 3 should be default (0).
        assert_eq!(buf.get_voxel(0, 0, 0, ChannelId::Type.index()), 7);
        assert_eq!(buf.get_voxel(0, 2, 0, ChannelId::Type.index()), 7);
        assert_eq!(buf.get_voxel(0, 3, 0, ChannelId::Type.index()), 0);
        assert_eq!(buf.get_voxel(0, 7, 0, ChannelId::Type.index()), 0);
    }

    // ---- Flat: early-exit branches --------------------------------------

    #[test]
    fn flat_early_exit_above_ground_leaves_air() {
        let mut gen = Flat::default();
        gen.height = 0.0;
        let mut buf = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        let result = gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: Vector3i::new(0, 100, 0), // well above the plane
            lod: 0,
        });
        assert!(result.max_lod_hint);
        // Buffer untouched: stays at default uniform value (0).
        assert_eq!(
            buf.channel_compression(ChannelId::Sdf.index()),
            Compression::Uniform
        );
    }

    #[test]
    fn flat_early_exit_below_ground_fills_uniform_sdf() {
        let mut gen = Flat::default();
        gen.height = 0.0;
        let mut buf = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        // 32-bit depth so the -100 sentinel survives the round-trip
        // (Bit16 SDF is quantized to [-1,1] via snorm and would saturate).
        buf.set_channel_depth(ChannelId::Sdf.index(), crate::storage::ChannelDepth::Bit32);
        let result = gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: Vector3i::new(0, -200, 0), // well below the plane
            lod: 0,
        });
        assert!(result.max_lod_hint);
        // SDF below ground is the C++ "not consistent" sentinel -100.
        let v = buf.get_voxel_f(0, 0, 0, ChannelId::Sdf.index());
        assert!(
            (v - (-100.0)).abs() < 1e-3,
            "below-ground SDF sentinel: {v}"
        );
    }

    #[test]
    fn flat_early_exit_below_ground_fills_uniform_blocky() {
        let mut gen = Flat::default();
        gen.channel = ChannelId::Type;
        gen.voxel_type = 9;
        gen.height = 0.0;
        let mut buf = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: Vector3i::new(0, -200, 0),
            lod: 0,
        });
        assert_eq!(buf.get_voxel(0, 0, 0, ChannelId::Type.index()), 9);
    }

    // ---- used_channels_mask ---------------------------------------------

    #[test]
    fn flat_used_channels_mask_reflects_configured_channel() {
        let mut gen = Flat::default();
        assert_eq!(gen.used_channels_mask(), 1 << ChannelId::Sdf.index());
        gen.set_channel(ChannelId::Type);
        assert_eq!(gen.used_channels_mask(), 1 << ChannelId::Type.index());
    }

    #[test]
    fn waves_used_channels_mask_defaults_to_sdf() {
        let gen = Waves::default();
        // Waves uses the shared heightmap helper, which always writes the
        // channel from HeightmapParams (default SDF).
        let g: &dyn VoxelGenerator = &gen;
        assert_eq!(g.used_channels_mask(), 1 << ChannelId::Sdf.index());
    }

    // ---- heightmap range remap (via Waves integration) ------------------

    #[test]
    fn waves_applies_height_range_remap() {
        // Default Waves: height_range = 30, height_start = 0.
        // At a peak (h≈1) the world height is ~30; at a trough (h≈0) it's ~0.
        let mut gen = Waves::default();
        gen.heightmap.height_start = 0.0;
        gen.heightmap.height_range = 30.0;
        gen.heightmap.iso_scale = 1.0;
        // Force a peak by placing the block where the sinusoid is maximal.
        // We can't easily pick a peak in integer coords, so just verify the
        // SDF at the very top of a tall block crosses zero somewhere — i.e.
        // the heightmap surface is inside the block, not above/below it.
        let mut buf = sdf_buffer(Vector3i::new(1, 40, 1));
        let origin = Vector3i::new(0, 0, 0);
        gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: origin,
            lod: 0,
        });
        // Find the sign-change row (the surface). Heights range ~[0,30], so the
        // crossing must be between y=0 and y=30.
        let mut found_crossing = false;
        for y in 0..39 {
            let a = buf.get_voxel_f(0, y, 0, ChannelId::Sdf.index());
            let b = buf.get_voxel_f(0, y + 1, 0, ChannelId::Sdf.index());
            if (a < 0.0) != (b < 0.0) {
                found_crossing = true;
                assert!(y < 30, "surface crossing at y={y} exceeds height_range 30");
                break;
            }
        }
        assert!(
            found_crossing,
            "no SDF sign change found; heightmap surface missing"
        );
    }

    // ---- heightmap offset (via Waves integration) -----------------------

    #[test]
    fn waves_heightmap_offset_shifts_origin() {
        let mut gen = Waves::default();
        gen.heightmap.offset = Vector2i::new(100, 0);
        let mut buf = sdf_buffer(Vector3i::new(1, 40, 1));
        gen.generate_block(VoxelQueryData {
            buffer: &mut buf,
            origin_in_voxels: Vector3i::new(0, 0, 0),
            lod: 0,
        });
        // With offset 100, sampling at world x=0 is the same as sampling at
        // x=-100 with no offset. Just verify the generator runs and produces a
        // crossing inside the height range (sanity check the offset path).
        let mut found_crossing = false;
        for y in 0..39 {
            let a = buf.get_voxel_f(0, y, 0, ChannelId::Sdf.index());
            let b = buf.get_voxel_f(0, y + 1, 0, ChannelId::Sdf.index());
            if (a < 0.0) != (b < 0.0) {
                found_crossing = true;
                break;
            }
        }
        assert!(found_crossing, "offset path produced no surface");
    }
}
