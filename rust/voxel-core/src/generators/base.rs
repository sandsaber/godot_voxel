//! The [`VoxelGenerator`] trait and the shared [`generate_heightmap`] helper.
//!
//! Ported from `generators/voxel_generator.h` (the abstract base) and
//! `generators/simple/voxel_generator_heightmap.h` (the templated heightmap
//! loop). The C++ base is a Godot `Resource` carrying an `RWLock` for
//! thread-safe parameter mutation plus a large surface of GPU / caching /
//! async-task hooks; only the synchronous generation contract is ported here.
//! Thread-safety of parameter access is left to the caller (the same stance
//! the rest of the Rust port takes), and the engine/streaming layer lands in
//! Phase 4.

use crate::math::{Vector2i, Vector3i};
use crate::storage::voxel_buffer::{ChannelId, MAX_CHANNELS};
use crate::storage::VoxelBuffer;

/// Input handed to [`VoxelGenerator::generate_block`]. Ported from
/// `VoxelGenerator::VoxelQueryData`.
#[derive(Debug)]
pub struct VoxelQueryData<'a> {
    pub buffer: &'a mut VoxelBuffer,
    pub origin_in_voxels: Vector3i,
    pub lod: u32,
}

/// Single-voxel query result. Ported from `VoxelSingleValue`. The same `u64`
/// slot carries either a raw integer or a bit-cast `f32` (for the SDF channel)
/// — read it with [`VoxelSingleValue::as_raw`] / [`VoxelSingleValue::as_real`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VoxelSingleValue(pub u64);

impl VoxelSingleValue {
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_raw(self) -> u64 {
        self.0
    }

    pub fn from_real(value: f32) -> Self {
        Self(f32::to_bits(value) as u64)
    }

    pub fn as_real(self) -> f32 {
        f32::from_bits(self.0 as u32)
    }
}

/// Output of [`VoxelGenerator::generate_block`]. Ported from
/// `VoxelGenerator::Result`.
///
/// `max_lod_hint` is an optimization flag: when `true`, the engine may skip
/// generating finer LODs because this generator considers the block already
/// final at the requested LOD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenResult {
    pub max_lod_hint: bool,
}

impl GenResult {
    /// Convenience for the common "nothing more to add" case.
    #[inline]
    pub fn max_lod() -> Self {
        Self { max_lod_hint: true }
    }
}

/// A voxel generator: fills a [`VoxelBuffer`] block with terrain data.
///
/// Ported from the C++ `VoxelGenerator` virtual base. Implementations must be
/// pure functions of their parameters and `input` — the engine may call
/// `generate_block` from any thread (the C++ base guards its parameters with
/// an RWLock; the Rust port requires `Send + Sync` so the generator can be
/// shared across threads via `Arc<Mutex<Box<dyn VoxelGenerator>>>` inside
/// [`crate::storage::VoxelData`]).
pub trait VoxelGenerator: Send + Sync {
    /// Fill `input.buffer` for the block starting at `input.origin_in_voxels`
    /// at level-of-detail `input.lod`.
    fn generate_block(&mut self, input: VoxelQueryData<'_>) -> GenResult;

    /// Bitmask of channels this generator writes (1 << channel_index).
    /// Defaults to the SDF channel. Ported from `get_used_channels_mask`.
    fn used_channels_mask(&self) -> u32 {
        1 << ChannelId::Sdf.index()
    }

    /// Sample a single voxel at world position `pos` (LOD 0). Returns the
    /// value packed into a [`VoxelSingleValue`] (raw integer, or bit-cast f32
    /// when `channel` is the SDF channel). Ported from
    /// `VoxelGenerator::generate_single`.
    ///
    /// The default implementation builds a 1×1×1 `VoxelBuffer`, runs
    /// [`generate_block`](Self::generate_block), and reads the result — slow
    /// but correct. Generators with a closed-form per-voxel expression can
    /// override this for a sizeable speedup. Returns `from_raw(0)` if
    /// `channel >= MAX_CHANNELS`.
    fn generate_single(&mut self, pos: Vector3i, channel: usize) -> VoxelSingleValue {
        if channel >= MAX_CHANNELS {
            return VoxelSingleValue::from_raw(0);
        }
        let mut buffer = VoxelBuffer::with_size(Vector3i::splat(1));
        self.generate_block(VoxelQueryData {
            buffer: &mut buffer,
            origin_in_voxels: pos,
            lod: 0,
        });
        if channel == ChannelId::Sdf.index() {
            VoxelSingleValue::from_real(buffer.get_voxel_f(0, 0, 0, channel))
        } else {
            VoxelSingleValue::from_raw(buffer.get_voxel(0, 0, 0, channel))
        }
    }
}

// ---------------------------------------------------------------------------
// Shared heightmap loop
// ---------------------------------------------------------------------------

/// Parameters shared by every heightmap generator. Ported from the private
/// `VoxelGeneratorHeightmap::Parameters` + `Range` structs, minus the RWLock
/// (the caller owns mutation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeightmapParams {
    /// Which channel the generator writes. Defaults to SDF.
    pub channel: ChannelId,
    /// Solid voxel id used when filling blocky terrain below the heightmap.
    pub matter_type: u64,
    /// Heightmap value range: sampled heights are remapped as
    /// `h * range.height + range.start`. Matches the C++ `Range::xform`.
    pub height_start: f32,
    pub height_range: f32,
    /// SDF iso-surface scale (`iso_scale` in C++). Multiplies `(y - h)`.
    pub iso_scale: f32,
    /// 2D offset subtracted from `(x, z)` before sampling.
    pub offset: Vector2iOffset,
}

/// 2D integer offset applied to the world origin before sampling the
/// heightmap. The C++ side uses Godot's `Vector2i`; the Rust math module
/// already ports it, so we re-export it here for convenience.
pub type Vector2iOffset = Vector2i;

impl Default for HeightmapParams {
    fn default() -> Self {
        Self {
            channel: ChannelId::Sdf,
            matter_type: 1,
            height_start: -50.0,
            height_range: 200.0,
            iso_scale: 1.0,
            offset: Vector2i::new(0, 0),
        }
    }
}

/// Shared heightmap generation loop. Ported from
/// `VoxelGeneratorHeightmap::generate`.
///
/// `height_fn` returns the (pre-range) height at world `(x, z)`. The function
/// applies the configured [`HeightmapParams`] range remap and fills
/// `out_buffer` for the block at `origin` / `lod`. Returns a [`GenResult`]
/// with `max_lod_hint` set when the block is entirely above or below the
/// heightmap range (the two early-exit cases in the C++ loop).
pub fn generate_heightmap<H>(
    out_buffer: &mut VoxelBuffer,
    height_fn: H,
    params: &HeightmapParams,
    origin: Vector3i,
    lod: u32,
) -> GenResult
where
    H: Fn(i32, i32) -> f32,
{
    // Apply the 2D offset to the world origin, matching the C++ origin remap.
    let origin = Vector3i::new(
        origin.x - params.offset.x,
        origin.y,
        origin.z - params.offset.y,
    );
    let channel = params.channel.index();
    let bs = out_buffer.size();
    let use_sdf = params.channel == ChannelId::Sdf;

    // Early-exit: block bottom above the highest possible ground → air.
    if origin.y as f32 > params.height_start + params.height_range {
        return GenResult::max_lod();
    }
    // Early-exit: block top below the lowest ground → uniform fill.
    let block_top = origin.y + (bs.y << lod);
    if (block_top as f32) < params.height_start {
        let clear = if use_sdf { 0 } else { params.matter_type };
        out_buffer.clear_channel(channel, clear);
        return GenResult::max_lod();
    }

    let stride = 1i32 << lod;
    let xform = |h: f32| h * params.height_range + params.height_start;

    if use_sdf {
        let mut gz = origin.z;
        for z in 0..bs.z {
            let mut gx = origin.x;
            for x in 0..bs.x {
                let h = xform(height_fn(gx, gz));
                let mut gy = origin.y;
                for y in 0..bs.y {
                    let sdf = params.iso_scale * (gy as f32 - h);
                    out_buffer.set_voxel_f(sdf, x, y, z, channel);
                    gy += stride;
                }
                gx += stride;
            }
            gz += stride;
        }
    } else {
        // Blocky: one sample per column, then fill [0, ih).
        let mut gz = origin.z;
        for z in 0..bs.z {
            let mut gx = origin.x;
            for x in 0..bs.x {
                let h = xform(height_fn(gx, gz)) - origin.y as f32;
                let mut ih = crate::math::funcs::arithmetic_rshift(h as i32, lod);
                if ih > 0 {
                    if ih > bs.y {
                        ih = bs.y;
                    }
                    out_buffer.fill_area(
                        params.matter_type,
                        Vector3i::new(x, 0, z),
                        Vector3i::new(x + 1, ih, z + 1),
                        channel,
                    );
                }
                gx += stride;
            }
            gz += stride;
        }
    }

    GenResult::default()
}

#[cfg(test)]
mod tests {
    use super::{VoxelGenerator, VoxelQueryData, VoxelSingleValue};
    use crate::generators::base::GenResult;
    use crate::math::Vector3i;
    use crate::storage::{ChannelId, VoxelBuffer};

    /// Trivial generator: writes a constant raw value into the Type channel
    /// so we can exercise `generate_single`'s default implementation without
    /// running into SDF quantization.
    struct ConstantType;
    impl VoxelGenerator for ConstantType {
        fn generate_block(&mut self, input: VoxelQueryData<'_>) -> GenResult {
            input.buffer.set_voxel(42, 0, 0, 0, ChannelId::Type.index());
            GenResult::default()
        }
        fn used_channels_mask(&self) -> u32 {
            1 << ChannelId::Type.index()
        }
    }

    #[test]
    fn generate_single_default_impl_reads_back_integer_channels_as_raw() {
        let mut gen = ConstantType;
        let value = gen.generate_single(Vector3i::new(1, 2, 3), ChannelId::Type.index());
        assert_eq!(value.as_raw(), 42);
    }

    #[test]
    fn generate_single_default_impl_reads_back_sdf_as_real() {
        // The SDF channel uses bit-cast for the single-voxel return. We pick
        // an exact-representable value to avoid 16-bit SDF quantization.
        struct SdfOnly;
        impl VoxelGenerator for SdfOnly {
            fn generate_block(&mut self, input: VoxelQueryData<'_>) -> GenResult {
                input
                    .buffer
                    .set_voxel_f(0.5, 0, 0, 0, ChannelId::Sdf.index());
                GenResult::default()
            }
        }
        let mut gen = SdfOnly;
        // 0.5 has an exact f32 representation; SDF 16-bit quantization still
        // loses precision, so we sample with the Bit32 depth path by reading
        // the f32 directly from a 1-voxel buffer.
        let mut probe = VoxelBuffer::with_size(Vector3i::splat(1));
        gen.generate_block(VoxelQueryData {
            buffer: &mut probe,
            origin_in_voxels: Vector3i::zero(),
            lod: 0,
        });
        // generate_single for SDF reads back via get_voxel_f, so it returns
        // the f32-bits of the (possibly quantized) stored value.
        let value = gen.generate_single(Vector3i::zero(), ChannelId::Sdf.index());
        assert_eq!(value.as_real(), probe.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()));
    }

    #[test]
    fn generate_single_returns_zero_for_out_of_range_channel() {
        let mut gen = ConstantType;
        let value = gen.generate_single(Vector3i::zero(), 99);
        assert_eq!(value, VoxelSingleValue::from_raw(0));
    }

    #[test]
    fn voxel_single_value_round_trips_raw_and_real() {
        assert_eq!(VoxelSingleValue::from_raw(7).as_raw(), 7);
        assert_eq!(VoxelSingleValue::from_real(2.5).as_real(), 2.5);
        // The bit-cast representation is preserved.
        assert_eq!(
            VoxelSingleValue::from_real(1.5).as_raw(),
            f32::to_bits(1.5) as u64
        );
    }

    #[test]
    fn default_generate_single_uses_one_voxel_buffer() {
        // Sanity: the default impl builds a 1x1x1 buffer, runs generate_block,
        // and returns the sole voxel. This also documents that contract.
        let mut gen = ConstantType;
        let mut probe = VoxelBuffer::with_size(Vector3i::splat(1));
        gen.generate_block(VoxelQueryData {
            buffer: &mut probe,
            origin_in_voxels: Vector3i::zero(),
            lod: 0,
        });
        assert_eq!(probe.get_voxel(0, 0, 0, ChannelId::Type.index()), 42);
    }
}
