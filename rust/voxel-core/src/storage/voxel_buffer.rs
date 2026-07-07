//! Dense multi-channel voxel storage.
//!
//! Ported from `storage/voxel_buffer.{h,cpp}`. Up to 8 channels of variable
//! bit-depth (8/16/32/64-bit), two compression modes (`NONE` — fully allocated,
//! `UNIFORM` — single default value, no allocation), ZXY memory layout
//! (`index = y + sy*(x + sx*z)`, Y innermost). The C++ class has two allocators
//! (DEFAULT = malloc, POOL = `VoxelMemoryPool`); in Rust both map to `Vec<u8>`,
//! and the pool is opt-in via [`VoxelMemoryPool`].
//!
//! ## `QUANTIZED_SDF_*` constants
//!
//! `voxel_buffer.h` references `constants::QUANTIZED_SDF_{8,16}_BITS_SCALE[_INV]`,
//! which scale SDF floats into the `[-1,1]` snorm range before 8/16-bit
//! quantization. The C++ constants intentionally give 8-bit SDF about
//! `[-10, 10]` range and 16-bit SDF about `[-500, 500]` range.

use super::depth::ChannelDepth;
use super::funcs;
use super::voxel_memory_pool::VoxelMemoryPool;
use crate::math::Vector3i;
use std::sync::Arc;

/// Number of channels. Matches `MAX_CHANNELS`. Indexed by [`ChannelId`].
pub const MAX_CHANNELS: usize = 8;
/// Mask selecting all channels. Matches `ALL_CHANNELS_MASK`.
pub const ALL_CHANNELS_MASK: u8 = 0xff;
/// Maximum size along any axis. Matches `MAX_SIZE`.
pub const MAX_SIZE: u32 = 65535;

/// SDF quantization scale for 8-bit channels. Matches `QUANTIZED_SDF_8_BITS_SCALE`.
/// `raw = snorm_to_s8(sdf * SCALE)`;
/// `sdf = s8_to_snorm(raw) * SCALE_INV`.
pub const QUANTIZED_SDF_8_BITS_SCALE: f32 = 0.1;
/// Inverse of [`QUANTIZED_SDF_8_BITS_SCALE`].
pub const QUANTIZED_SDF_8_BITS_SCALE_INV: f32 = 1.0 / QUANTIZED_SDF_8_BITS_SCALE;
/// SDF quantization scale for 16-bit channels. Matches `QUANTIZED_SDF_16_BITS_SCALE`.
pub const QUANTIZED_SDF_16_BITS_SCALE: f32 = 0.002;
/// Inverse of [`QUANTIZED_SDF_16_BITS_SCALE`].
pub const QUANTIZED_SDF_16_BITS_SCALE_INV: f32 = 1.0 / QUANTIZED_SDF_16_BITS_SCALE;

/// Matches `constants::SDF_FAR_OUTSIDE`.
pub const SDF_FAR_OUTSIDE: f32 = 100.0;
/// Matches `constants::SDF_FAR_INSIDE`.
pub const SDF_FAR_INSIDE: f32 = -100.0;

/// Matches `mixel4::encode_indices_to_packed_u16(0, 1, 2, 3)`.
pub const MIXEL4_DEFAULT_INDICES: u64 = 0x3210;
/// Matches `mixel4::encode_weights_to_packed_u16_lossy(255, 0, 0, 0)`.
pub const MIXEL4_DEFAULT_WEIGHTS: u64 = 0x000f;

/// Identifies a channel within a [`VoxelBuffer`]. Matches `ChannelId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChannelId {
    /// Block type / material id.
    Type = 0,
    /// Signed distance field (for smooth meshing).
    Sdf = 1,
    /// Per-voxel color.
    Color = 2,
    /// Material indices (4-way blend).
    Indices = 3,
    /// Material blend weights.
    Weights = 4,
    /// Free-form data channel 5.
    Data5 = 5,
    /// Free-form data channel 6.
    Data6 = 6,
    /// Free-form data channel 7.
    Data7 = 7,
}

impl ChannelId {
    /// Human-readable name. Matches `get_channel_name`.
    pub fn name(self) -> &'static str {
        match self {
            ChannelId::Type => "type",
            ChannelId::Sdf => "sdf",
            ChannelId::Color => "color",
            ChannelId::Indices => "indices",
            ChannelId::Weights => "weights",
            ChannelId::Data5 => "data5",
            ChannelId::Data6 => "data6",
            ChannelId::Data7 => "data7",
        }
    }

    /// Convert to the channel index `0..MAX_CHANNELS`.
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }
}

/// How a channel's voxels are stored. Matches `Compression`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Compression {
    /// Fully allocated array.
    None = 0,
    /// Single uniform default value; no array allocated.
    Uniform = 1,
}

/// Allocator strategy. Matches `Allocator`. In Rust both use `Vec<u8>`; `Pool`
/// routes fresh allocations through an optional [`VoxelMemoryPool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Allocator {
    /// `malloc`-backed (the default).
    Default = 0,
    /// [`VoxelMemoryPool`] recycling (faster for many same-sized buffers).
    Pool = 1,
}

impl ChannelDepth {
    /// Number of bytes per voxel for this depth. Matches `get_depth_byte_count`.
    #[inline]
    pub fn byte_count(self) -> u32 {
        1u32 << (self as u32)
    }

    /// Number of bits per voxel. Matches `get_depth_bit_count`.
    #[inline]
    pub fn bit_count(self) -> u32 {
        self.byte_count() << 3
    }
}

/// Default depth constants matching the C++ `DEFAULT_*_CHANNEL_DEPTH`.
pub const DEFAULT_CHANNEL_DEPTH: ChannelDepth = ChannelDepth::Bit8;
pub const DEFAULT_TYPE_CHANNEL_DEPTH: ChannelDepth = ChannelDepth::Bit16;
pub const DEFAULT_SDF_CHANNEL_DEPTH: ChannelDepth = ChannelDepth::Bit16;
pub const DEFAULT_INDICES_CHANNEL_DEPTH: ChannelDepth = ChannelDepth::Bit16;
pub const DEFAULT_WEIGHTS_CHANNEL_DEPTH: ChannelDepth = ChannelDepth::Bit16;

/// Get the default raw (integer) value for a channel at the given depth.
/// Matches `VoxelBuffer::get_default_raw_value`.
pub fn get_default_raw_value(channel: ChannelId, depth: ChannelDepth) -> u64 {
    match channel {
        ChannelId::Type => 0,
        ChannelId::Sdf => get_default_sdf_raw_value(depth),
        ChannelId::Indices => get_default_indices_raw_value(depth),
        ChannelId::Weights => MIXEL4_DEFAULT_WEIGHTS,
        ChannelId::Color | ChannelId::Data5 | ChannelId::Data6 | ChannelId::Data7 => 0,
    }
}

/// Default SDF raw value at `depth`: far outside/air. Matches
/// `get_default_sdf_raw_value`.
pub fn get_default_sdf_raw_value(depth: ChannelDepth) -> u64 {
    match depth {
        ChannelDepth::Bit8 => funcs::snorm_to_s8(1.0) as u8 as u64,
        ChannelDepth::Bit16 => funcs::snorm_to_s16(1.0) as u16 as u64,
        ChannelDepth::Bit32 => f32::to_bits(SDF_FAR_OUTSIDE) as u64,
        ChannelDepth::Bit64 => f64::to_bits(SDF_FAR_OUTSIDE as f64),
    }
}

/// Default SDF float value at `depth`. Matches the decoded C++ default.
pub fn get_default_sdf_value(depth: ChannelDepth) -> f32 {
    raw_voxel_to_real(get_default_sdf_raw_value(depth), depth)
}

/// Default indices raw value at `depth`: material slots 0,1,2,3. Matches
/// `get_default_indices_raw_value`.
pub fn get_default_indices_raw_value(_depth: ChannelDepth) -> u64 {
    MIXEL4_DEFAULT_INDICES
}

/// Convert a float to a raw (integer) voxel value at `depth`. Matches
/// `real_to_raw_voxel`. 8/16-bit quantize to snorm × SDF scale; 32/64-bit store
/// the float bits directly.
pub fn real_to_raw_voxel(value: f32, depth: ChannelDepth) -> u64 {
    match depth {
        ChannelDepth::Bit8 => funcs::snorm_to_s8(value * QUANTIZED_SDF_8_BITS_SCALE) as u8 as u64,
        ChannelDepth::Bit16 => {
            funcs::snorm_to_s16(value * QUANTIZED_SDF_16_BITS_SCALE) as u16 as u64
        }
        ChannelDepth::Bit32 => f32::to_bits(value) as u64,
        ChannelDepth::Bit64 => f64::to_bits(value as f64),
    }
}

/// Convert a raw (integer) voxel value at `depth` back to float. Matches
/// `raw_voxel_to_real`. 8/16-bit expand from snorm × SDF scale.
pub fn raw_voxel_to_real(value: u64, depth: ChannelDepth) -> f32 {
    match depth {
        ChannelDepth::Bit8 => {
            funcs::s8_to_snorm(value as u8 as i8) * QUANTIZED_SDF_8_BITS_SCALE_INV
        }
        ChannelDepth::Bit16 => {
            funcs::s16_to_snorm(value as u16 as i16) * QUANTIZED_SDF_16_BITS_SCALE_INV
        }
        ChannelDepth::Bit32 => f32::from_bits(value as u32),
        ChannelDepth::Bit64 => f64::from_bits(value) as f32,
    }
}

/// One channel's storage. Matches `VoxelBuffer::Channel`. Either a uniform
/// default value (`Compression::Uniform`, no allocation) or a fully-allocated
/// byte array (`Compression::None`).
#[derive(Debug)]
pub struct Channel {
    /// Allocated voxel data. Present only when `compression == None`.
    /// ZXY layout, length = `size_in_bytes`.
    pub data: Vec<u8>,
    /// Default value when uniform (encoded; use [`raw_voxel_to_real`] to decode).
    pub defval: u64,
    pub depth: ChannelDepth,
    pub compression: Compression,
    /// Allocated bytes (= volume * depth.byte_count()) when `None`.
    pub size_in_bytes: u32,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            defval: 0,
            depth: DEFAULT_CHANNEL_DEPTH,
            compression: Compression::Uniform,
            size_in_bytes: 0,
        }
    }
}

impl Channel {
    /// Bytes needed to store a buffer of `size` at `depth`. Matches
    /// `get_size_in_bytes_for_volume`.
    pub fn size_in_bytes_for_volume(size: Vector3i, depth: ChannelDepth) -> usize {
        (size.x as usize) * (size.y as usize) * (size.z as usize) * depth.byte_count() as usize
    }
}

/// Dense multi-channel voxel buffer. The main Phase-3 storage type, replacing
/// the pilot's single-channel [`super::buffer::DenseVoxelBuffer`] for general use.
///
/// Owned storage. Channels start in `Compression::Uniform` and allocate on first
/// write. When the `pool` allocator is chosen, fresh allocations route through
/// the shared [`VoxelMemoryPool`] (if one is attached).
pub struct VoxelBuffer {
    size: Vector3i,
    channels: [Channel; MAX_CHANNELS],
    allocator: Allocator,
    /// Optional pool used when `allocator == Pool`.
    pool: Option<Arc<VoxelMemoryPool>>,
}

impl std::fmt::Debug for VoxelBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoxelBuffer")
            .field("size", &self.size)
            .field("allocator", &self.allocator)
            .field("pool_attached", &self.pool.is_some())
            .finish()
    }
}

impl VoxelBuffer {
    /// Create an empty (zero-size) buffer with the given allocator. Matches the
    /// C++ `VoxelBuffer(Allocator)` ctor. Call [`create`](Self::create) to size it.
    pub fn new(allocator: Allocator) -> Self {
        Self {
            size: Vector3i::zero(),
            channels: std::array::from_fn(default_channel_for_index),
            allocator,
            pool: None,
        }
    }

    /// Create with a `Default` allocator and the given size; channels use the
    /// engine's default per-channel depths and uniform defaults. Convenience
    /// over [`new`](Self::new) + [`create`](Self::create).
    pub fn with_size(size: Vector3i) -> Self {
        let mut vb = Self::new(Allocator::Default);
        vb.create(size);
        vb
    }

    /// Attach a memory pool for `Allocator::Pool`. Has no effect for `Default`.
    pub fn with_pool(mut self, pool: Arc<VoxelMemoryPool>) -> Self {
        self.pool = Some(pool);
        self
    }

    /// (Re)allocate to `size` voxels. Every channel is reset to uniform at the
    /// default value for its current depth. Matches the C++ behavior when
    /// `new_format` is null: channel depths are preserved unless a caller
    /// explicitly applies a [`VoxelFormat`](crate::storage::VoxelFormat).
    pub fn create(&mut self, size: Vector3i) {
        debug_assert!(size.x >= 0 && size.y >= 0 && size.z >= 0);
        debug_assert!(
            (size.x as u32) <= MAX_SIZE
                && (size.y as u32) <= MAX_SIZE
                && (size.z as u32) <= MAX_SIZE
        );
        self.size = size;
        for (i, ch) in self.channels.iter_mut().enumerate() {
            free_channel_data(self.allocator, self.pool.as_ref(), ch);
            ch.compression = Compression::Uniform;
            ch.defval = get_default_raw_value(channel_id_from_index(i).unwrap(), ch.depth);
        }
    }

    /// Size in voxels.
    #[inline]
    pub fn size(&self) -> Vector3i {
        self.size
    }

    /// Which allocator this buffer uses.
    #[inline]
    pub fn allocator(&self) -> Allocator {
        self.allocator
    }

    /// Depth of a channel.
    #[inline]
    pub fn channel_depth(&self, channel_index: usize) -> ChannelDepth {
        self.channels[channel_index].depth
    }

    /// Set the depth of a channel. Matches `set_channel_depth`: changing an
    /// allocated channel resets it to a uniform default because existing bytes
    /// no longer match the new element width.
    pub fn set_channel_depth(&mut self, channel_index: usize, depth: ChannelDepth) {
        if self.channels[channel_index].depth == depth {
            return;
        }
        let channel_id = channel_id_from_index(channel_index).unwrap();
        let ch = &mut self.channels[channel_index];
        free_channel_data(self.allocator, self.pool.as_ref(), ch);
        ch.depth = depth;
        ch.defval = get_default_raw_value(channel_id, depth);
        ch.compression = Compression::Uniform;
    }

    /// Compression of a channel. Matches `get_channel_compression`.
    #[inline]
    pub fn channel_compression(&self, channel_index: usize) -> Compression {
        self.channels[channel_index].compression
    }

    /// Reset a channel to a uniform raw value, freeing its allocation.
    /// Matches `clear_channel`.
    pub fn clear_channel(&mut self, channel_index: usize, clear_value: u64) {
        let ch = &mut self.channels[channel_index];
        free_channel_data(self.allocator, self.pool.as_ref(), ch);
        ch.defval = clear_value;
        ch.compression = Compression::Uniform;
    }

    /// Reset a channel to a uniform float value. Matches `clear_channel_f`.
    pub fn clear_channel_f(&mut self, channel_index: usize, clear_value: f32) {
        let depth = self.channels[channel_index].depth;
        self.clear_channel(channel_index, real_to_raw_voxel(clear_value, depth));
    }

    /// Get a voxel as a raw `u64` at `depth` width. Matches `get_voxel`.
    pub fn get_voxel(&self, x: i32, y: i32, z: i32, channel_index: usize) -> u64 {
        let ch = &self.channels[channel_index];
        if ch.compression == Compression::Uniform {
            return ch.defval;
        }
        let i = voxel_index(self.size, x as usize, y as usize, z as usize);
        read_raw(&ch.data, i, ch.depth)
    }

    /// Set a voxel from a raw `u64`. Matches `set_voxel`. Decompresses the
    /// channel on first write into it.
    pub fn set_voxel(&mut self, value: u64, x: i32, y: i32, z: i32, channel_index: usize) {
        self.decompress_channel(channel_index);
        let depth = self.channels[channel_index].depth;
        let ch = &mut self.channels[channel_index];
        let i = voxel_index(self.size, x as usize, y as usize, z as usize);
        write_raw(&mut ch.data, i, depth, value);
    }

    /// Get a voxel as float. Matches `get_voxel_f`.
    pub fn get_voxel_f(&self, x: i32, y: i32, z: i32, channel_index: usize) -> f32 {
        let raw = self.get_voxel(x, y, z, channel_index);
        raw_voxel_to_real(raw, self.channels[channel_index].depth)
    }

    /// Set a voxel from float. Matches `set_voxel_f`.
    pub fn set_voxel_f(&mut self, value: f32, x: i32, y: i32, z: i32, channel_index: usize) {
        let depth = self.channels[channel_index].depth;
        self.set_voxel(real_to_raw_voxel(value, depth), x, y, z, channel_index);
    }

    /// Fill an entire channel with a raw value. Matches `fill`.
    pub fn fill(&mut self, value: u64, channel_index: usize) {
        // If the value equals the current uniform default, stay compressed.
        let ch = &mut self.channels[channel_index];
        if ch.compression == Compression::Uniform && ch.defval == value {
            return;
        }
        // Otherwise: become uniform with this value (the simplest faithful fill
        // — the C++ also leaves uniform channels uniform when fill == defval,
        // and only allocates for non-uniform fills via the per-voxel loop).
        free_channel_data(self.allocator, self.pool.as_ref(), ch);
        ch.defval = value;
        ch.compression = Compression::Uniform;
    }

    /// Fill a rectangular area with a raw value. Matches `fill_area`. Always
    /// decompresses the channel.
    pub fn fill_area(&mut self, value: u64, min: Vector3i, max: Vector3i, channel_index: usize) {
        self.decompress_channel(channel_index);
        let depth = self.channels[channel_index].depth;
        let bytes = depth.byte_count() as usize;
        let ch = &mut self.channels[channel_index];
        let size = self.size;
        // Fill row-by-row in byte form. We can't use fill_3d_region_zxy< T >
        // generically, so cast the data slice to a per-element layout via bytes.
        let mut lo = min;
        let mut hi = max;
        crate::math::Vector3i::sort_min_max(&mut lo, &mut hi);
        lo.x = crate::math::funcs::clamp(lo.x, 0, size.x);
        lo.y = crate::math::funcs::clamp(lo.y, 0, size.y);
        lo.z = crate::math::funcs::clamp(lo.z, 0, size.z);
        hi.x = crate::math::funcs::clamp(hi.x, 0, size.x);
        hi.y = crate::math::funcs::clamp(hi.y, 0, size.y);
        hi.z = crate::math::funcs::clamp(hi.z, 0, size.z);
        let area = hi - lo;
        if area.x <= 0 || area.y <= 0 || area.z <= 0 {
            return;
        }
        let le = encode_raw(value, depth);
        for z in 0..area.z {
            for x in 0..area.x {
                for y in 0..area.y {
                    let i = voxel_index(
                        size,
                        (lo.x + x) as usize,
                        (lo.y + y) as usize,
                        (lo.z + z) as usize,
                    );
                    ch.data[i * bytes..i * bytes + bytes].copy_from_slice(&le[..bytes]);
                }
            }
        }
    }

    /// True if a channel is uniform (all voxels equal its default). Matches
    /// `is_uniform`. Compressed channels are uniform by definition.
    pub fn is_uniform(&self, channel_index: usize) -> bool {
        let ch = &self.channels[channel_index];
        if ch.compression == Compression::Uniform {
            return true;
        }
        let bytes = ch.depth.byte_count() as usize;
        let first = &ch.data[..bytes];
        ch.data.chunks_exact(bytes).all(|c| c == first)
    }

    /// Decompress a channel (allocate and fill with its default). No-op if
    /// already `NONE`. Matches `decompress_channel`.
    pub fn decompress_channel(&mut self, channel_index: usize) {
        // Snapshot the channel's immutable fields before the mutable borrow, to
        // avoid holding `&mut channel` across `self.alloc(...)`.
        let (compression, depth, defval) = {
            let ch = &self.channels[channel_index];
            (ch.compression, ch.depth, ch.defval)
        };
        if compression == Compression::None {
            return;
        }
        let bytes_needed = Channel::size_in_bytes_for_volume(self.size, depth);
        let mut data = self.alloc(bytes_needed);
        let le = encode_raw(defval, depth);
        let unit = depth.byte_count() as usize;
        for chunk in data.chunks_exact_mut(unit) {
            chunk.copy_from_slice(&le[..unit]);
        }
        let ch = &mut self.channels[channel_index];
        ch.data = data;
        ch.size_in_bytes = bytes_needed as u32;
        ch.compression = Compression::None;
    }

    /// Compress any channel whose voxels are all equal into a uniform default.
    /// Matches `compress_uniform_channels`.
    pub fn compress_uniform_channels(&mut self) {
        for ci in 0..MAX_CHANNELS {
            if self.channels[ci].compression == Compression::None && self.is_uniform(ci) {
                let depth = self.channels[ci].depth;
                let defval = read_raw(&self.channels[ci].data, 0, depth);
                let ch = &mut self.channels[ci];
                free_channel_data(self.allocator, self.pool.as_ref(), ch);
                ch.defval = defval;
                ch.compression = Compression::Uniform;
            }
        }
    }

    /// Raw bytes of a channel (decompressed). Matches `get_channel_as_bytes`.
    /// Decompresses if needed.
    pub fn channel_bytes_mut(&mut self, channel_index: usize) -> &mut [u8] {
        self.decompress_channel(channel_index);
        &mut self.channels[channel_index].data
    }

    /// Raw bytes of a channel (read-only). If compressed, returns an empty slice
    /// — callers should use `defval` in that case. For a guaranteed-materialized
    /// view, call `decompress_channel` first.
    pub fn channel_bytes(&self, channel_index: usize) -> &[u8] {
        &self.channels[channel_index].data
    }

    /// The uniform default value of a channel.
    pub fn channel_default(&self, channel_index: usize) -> u64 {
        self.channels[channel_index].defval
    }

    /// Copy an entire channel from `other`. Matches `copy_channel_from`.
    pub fn copy_channel_from(&mut self, other: &VoxelBuffer, channel_index: usize) {
        assert_eq!(
            self.size, other.size,
            "copy_channel_from requires equal buffer sizes"
        );
        let src = &other.channels[channel_index];
        assert_eq!(
            self.channels[channel_index].depth, src.depth,
            "copy_channel_from requires equal channel depths"
        );

        if src.compression == Compression::None {
            let bytes = src.size_in_bytes as usize;
            let mut data = self.alloc(bytes);
            data[..bytes].copy_from_slice(&src.data[..bytes]);
            let dst = &mut self.channels[channel_index];
            free_channel_data(self.allocator, self.pool.as_ref(), dst);
            dst.defval = src.defval;
            dst.compression = Compression::None;
            dst.size_in_bytes = src.size_in_bytes;
            dst.data = data;
            return;
        }

        let dst = &mut self.channels[channel_index];
        free_channel_data(self.allocator, self.pool.as_ref(), dst);
        dst.defval = src.defval;
        dst.compression = Compression::Uniform;
    }

    /// Copy all voxel data and channel depths into `dst`. Metadata is not part
    /// of the Rust storage port yet, so this matches the voxel-data half of C++
    /// `copy_to`.
    pub fn copy_to(&self, dst: &mut VoxelBuffer) {
        dst.create(self.size);
        for ci in 0..MAX_CHANNELS {
            dst.set_channel_depth(ci, self.channels[ci].depth);
        }
        dst.copy_channels_from(self);
    }

    pub fn copy_to_owned(&self) -> VoxelBuffer {
        let mut dst = VoxelBuffer::new(self.allocator);
        if let Some(pool) = &self.pool {
            dst = dst.with_pool(pool.clone());
        }
        self.copy_to(&mut dst);
        dst
    }

    /// Copy a rectangular area of one channel from `other`. Matches the C++
    /// `copy_channel_from(other, src_min, src_max, dst_min, channel)` overload.
    pub fn copy_channel_from_area(
        &mut self,
        other: &VoxelBuffer,
        mut src_min: Vector3i,
        mut src_max: Vector3i,
        mut dst_min: Vector3i,
        channel_index: usize,
    ) {
        let src = &other.channels[channel_index];
        assert_eq!(
            self.channels[channel_index].depth, src.depth,
            "copy_channel_from_area requires equal channel depths"
        );

        Vector3i::sort_min_max(&mut src_min, &mut src_max);
        funcs::clip_copy_region(
            &mut src_min,
            &mut src_max,
            other.size,
            &mut dst_min,
            self.size,
        );
        let area_size = src_max - src_min;
        if area_size.x <= 0 || area_size.y <= 0 || area_size.z <= 0 {
            return;
        }

        if src.compression == Compression::None {
            if self.channels[channel_index].compression == Compression::Uniform {
                self.decompress_channel(channel_index);
            }
            let item_size = self.channels[channel_index].depth.byte_count() as usize;
            funcs::copy_3d_region_zxy(
                &mut self.channels[channel_index].data,
                self.size,
                dst_min,
                &src.data,
                other.size,
                src_min,
                src_max,
                item_size,
            );
            return;
        }

        if self.channels[channel_index].compression == Compression::Uniform
            && self.channels[channel_index].defval == src.defval
        {
            return;
        }

        self.fill_area(src.defval, dst_min, dst_min + area_size, channel_index);
    }

    /// Nearest-neighbor 2:1 downscale of all channels from a region of `self`
    /// into a region of `dst`. Matches `VoxelBuffer::downscale_to`.
    ///
    /// For each destination voxel `dst_pos`, the source voxel sampled is
    /// `src_min + ((dst_pos - dst_min) << 1)`. Channels that are uniform on
    /// both ends with equal defaults are skipped (no allocation, no writes).
    /// This is the mip-map kernel used by [`crate::storage::VoxelData`] to
    /// cascade edits up the LOD chain.
    pub fn downscale_to(
        &self,
        dst: &mut VoxelBuffer,
        mut src_min: Vector3i,
        mut src_max: Vector3i,
        mut dst_min: Vector3i,
    ) {
        // Clamp source region into this buffer.
        src_min = src_min.clamp(Vector3i::zero(), self.size - Vector3i::splat(1));
        src_max = src_max.clamp(Vector3i::zero(), self.size);

        let dst_max_raw = dst_min + ((src_max - src_min) >> 1);

        // Clamp destination region into `dst`.
        dst_min = dst_min.clamp(Vector3i::zero(), dst.size - Vector3i::splat(1));
        let dst_max = dst_max_raw.clamp(Vector3i::zero(), dst.size);

        for channel_index in 0..MAX_CHANNELS {
            let src_compression = self.channel_compression(channel_index);
            let dst_compression = dst.channel_compression(channel_index);
            let src_defval = self.channel_default(channel_index);
            let dst_defval = dst.channel_default(channel_index);

            // If both channels carry the same uniform default there is nothing
            // to do — the destination already matches. Matches the C++ fast path.
            if src_compression == Compression::Uniform
                && dst_compression == Compression::Uniform
                && src_defval == dst_defval
            {
                continue;
            }

            // ZXY iteration matches the C++ loop order so downscaled buffers
            // remain byte-comparable with the reference implementation.
            let mut dst_pos = dst_min;
            while dst_pos.z < dst_max.z {
                dst_pos.x = dst_min.x;
                while dst_pos.x < dst_max.x {
                    dst_pos.y = dst_min.y;
                    while dst_pos.y < dst_max.y {
                        let src_pos = src_min + ((dst_pos - dst_min) << 1);
                        // Source bounds were clamped above; verify defensively.
                        debug_assert!(src_pos.x >= 0 && src_pos.y >= 0 && src_pos.z >= 0);
                        debug_assert!(src_pos.x < self.size.x);
                        debug_assert!(src_pos.y < self.size.y);
                        debug_assert!(src_pos.z < self.size.z);

                        let value = if src_compression == Compression::Uniform {
                            src_defval
                        } else {
                            self.get_voxel(src_pos.x, src_pos.y, src_pos.z, channel_index)
                        };
                        dst.set_voxel(value, dst_pos.x, dst_pos.y, dst_pos.z, channel_index);
                        dst_pos.y += 1;
                    }
                    dst_pos.x += 1;
                }
                dst_pos.z += 1;
            }
        }
    }

    /// Copy all channels from `other`. Matches `copy_channels_from`.
    pub fn copy_channels_from(&mut self, other: &VoxelBuffer) {
        for ci in 0..MAX_CHANNELS {
            self.copy_channel_from(other, ci);
        }
    }

    // ---- internal helpers ----

    /// Allocate `n` bytes, via the pool if attached and `Pool` is selected.
    fn alloc(&self, n: usize) -> Vec<u8> {
        match (self.allocator, &self.pool) {
            (Allocator::Pool, Some(pool)) => {
                let mut v = pool.allocate(n);
                v.resize(n, 0);
                v
            }
            _ => vec![0u8; n],
        }
    }
}

/// Free a channel's data, returning it to `pool` if one is attached and the
/// buffer uses `Allocator::Pool`. Free function (not a method) to avoid
/// borrow conflicts when called while holding `&mut self.channels[i]`.
fn free_channel_data(allocator: Allocator, pool: Option<&Arc<VoxelMemoryPool>>, ch: &mut Channel) {
    if ch.data.is_empty() {
        ch.size_in_bytes = 0;
        return;
    }
    if matches!(allocator, Allocator::Pool) {
        if let Some(pool) = pool {
            pool.recycle(std::mem::take(&mut ch.data));
        } else {
            ch.data = Vec::new();
        }
    } else {
        ch.data = Vec::new();
    }
    ch.size_in_bytes = 0;
}

impl Drop for VoxelBuffer {
    fn drop(&mut self) {
        // Return pooled allocations on drop.
        if matches!(self.allocator, Allocator::Pool) {
            if let Some(pool) = &self.pool {
                for ch in &mut self.channels {
                    if !ch.data.is_empty() {
                        let data = std::mem::take(&mut ch.data);
                        pool.recycle(data);
                    }
                }
            }
        }
    }
}

// ---- free helpers ----

/// Index into a flat ZXY channel. Matches `get_index(x,y,z) = y + sy*(x + sx*z)`.
#[inline]
pub fn voxel_index(size: Vector3i, x: usize, y: usize, z: usize) -> usize {
    debug_assert!(x < size.x as usize && y < size.y as usize && z < size.z as usize);
    y + (size.y as usize) * (x + (size.x as usize) * z)
}

/// Read a little-endian raw value of `depth` width from `data` at voxel `i`.
#[inline]
fn read_raw(data: &[u8], i: usize, depth: ChannelDepth) -> u64 {
    let b = depth.byte_count() as usize;
    let off = i * b;
    match depth {
        ChannelDepth::Bit8 => data[off] as u64,
        ChannelDepth::Bit16 => u16::from_le_bytes([data[off], data[off + 1]]) as u64,
        ChannelDepth::Bit32 => {
            u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as u64
        }
        ChannelDepth::Bit64 => u64::from_le_bytes(data[off..off + 8].try_into().unwrap()),
    }
}

/// Write a little-endian raw value of `depth` width into `data` at voxel `i`.
#[inline]
fn write_raw(data: &mut [u8], i: usize, depth: ChannelDepth, value: u64) {
    let le = encode_raw(value, depth);
    let b = depth.byte_count() as usize;
    let off = i * b;
    data[off..off + b].copy_from_slice(&le[..b]);
}

/// Encode a raw value as little-endian bytes (8 bytes, truncated by depth).
#[inline]
fn encode_raw(value: u64, depth: ChannelDepth) -> [u8; 8] {
    // 64-bit LE covers all depths; callers slice the first `byte_count` bytes.
    let _ = depth;
    value.to_le_bytes()
}

/// Default depth for a channel at linear index `i` (matches DEFAULT_*_CHANNEL_DEPTH).
fn default_depth_for_channel_index(i: usize) -> ChannelDepth {
    match i {
        0 => DEFAULT_TYPE_CHANNEL_DEPTH,
        1 => DEFAULT_SDF_CHANNEL_DEPTH,
        3 => DEFAULT_INDICES_CHANNEL_DEPTH,
        4 => DEFAULT_WEIGHTS_CHANNEL_DEPTH,
        _ => DEFAULT_CHANNEL_DEPTH,
    }
}

fn default_channel_for_index(i: usize) -> Channel {
    let depth = default_depth_for_channel_index(i);
    Channel {
        depth,
        defval: get_default_raw_value(channel_id_from_index(i).unwrap(), depth),
        ..Default::default()
    }
}

/// Recover a `ChannelId` from a linear index, or `None` if out of range.
fn channel_id_from_index(i: usize) -> Option<ChannelId> {
    match i {
        0 => Some(ChannelId::Type),
        1 => Some(ChannelId::Sdf),
        2 => Some(ChannelId::Color),
        3 => Some(ChannelId::Indices),
        4 => Some(ChannelId::Weights),
        5 => Some(ChannelId::Data5),
        6 => Some(ChannelId::Data6),
        7 => Some(ChannelId::Data7),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_sets_defaults() {
        let vb = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        assert_eq!(vb.size(), Vector3i::new(4, 4, 4));
        // All channels uniform by default.
        for ci in 0..MAX_CHANNELS {
            assert_eq!(vb.channel_compression(ci), Compression::Uniform);
        }
        // Type channel at 16-bit, SDF at 16-bit, color at 8-bit.
        assert_eq!(
            vb.channel_depth(ChannelId::Type.index()),
            ChannelDepth::Bit16
        );
        assert_eq!(
            vb.channel_depth(ChannelId::Sdf.index()),
            ChannelDepth::Bit16
        );
        assert_eq!(
            vb.channel_depth(ChannelId::Color.index()),
            ChannelDepth::Bit8
        );
        assert_eq!(
            vb.channel_default(ChannelId::Indices.index()),
            0x3210,
            "C++ mixel4 default indices encode slots 0,1,2,3"
        );
        assert_eq!(
            vb.channel_default(ChannelId::Weights.index()),
            0x000f,
            "C++ mixel4 default weights encode full weight in slot 0"
        );
    }

    #[test]
    fn new_initializes_channel_defaults_before_create() {
        let vb = VoxelBuffer::new(Allocator::Default);
        assert_eq!(vb.size(), Vector3i::zero());
        assert_eq!(
            vb.channel_depth(ChannelId::Type.index()),
            ChannelDepth::Bit16
        );
        assert_eq!(
            vb.channel_depth(ChannelId::Sdf.index()),
            ChannelDepth::Bit16
        );
        assert_eq!(
            vb.channel_default(ChannelId::Sdf.index()),
            funcs::snorm_to_s16(1.0) as u16 as u64
        );
        assert_eq!(
            vb.channel_default(ChannelId::Indices.index()),
            MIXEL4_DEFAULT_INDICES
        );
        assert_eq!(
            vb.channel_default(ChannelId::Weights.index()),
            MIXEL4_DEFAULT_WEIGHTS
        );
    }

    #[test]
    fn create_preserves_existing_channel_depths() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        vb.set_channel_depth(ChannelId::Sdf.index(), ChannelDepth::Bit32);
        vb.set_channel_depth(ChannelId::Color.index(), ChannelDepth::Bit16);
        vb.set_voxel_f(-0.25, 0, 0, 0, ChannelId::Sdf.index());

        vb.create(Vector3i::new(3, 3, 3));

        assert_eq!(vb.size(), Vector3i::new(3, 3, 3));
        assert_eq!(
            vb.channel_depth(ChannelId::Sdf.index()),
            ChannelDepth::Bit32
        );
        assert_eq!(
            vb.channel_depth(ChannelId::Color.index()),
            ChannelDepth::Bit16
        );
        assert_eq!(
            vb.channel_compression(ChannelId::Sdf.index()),
            Compression::Uniform
        );
        assert_eq!(
            vb.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()),
            get_default_sdf_value(ChannelDepth::Bit32)
        );
    }

    #[test]
    fn uniform_get_returns_default() {
        let vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        // SDF channel is 16-bit by default; C++ defaults it to max positive
        // snorm, i.e. "far outside"/air, not solid.
        assert_eq!(
            vb.channel_default(ChannelId::Sdf.index()),
            funcs::snorm_to_s16(1.0) as u16 as u64
        );
        // Decoded through C++ QUANTIZED_SDF_16_BITS_SCALE_INV (500.0).
        let f = vb.get_voxel_f(0, 0, 0, ChannelId::Sdf.index());
        assert!(
            (f - 500.0).abs() < 1e-3,
            "SDF default decoded to {f}, want ~500.0"
        );
    }

    #[test]
    fn sdf_quantization_constants_match_cpp_ranges() {
        assert_eq!(QUANTIZED_SDF_8_BITS_SCALE, 0.1);
        assert_eq!(QUANTIZED_SDF_8_BITS_SCALE_INV, 10.0);
        assert_eq!(QUANTIZED_SDF_16_BITS_SCALE, 0.002);
        assert!((QUANTIZED_SDF_16_BITS_SCALE_INV - 500.0).abs() < 1e-3);

        assert_eq!(real_to_raw_voxel(10.0, ChannelDepth::Bit8), 127);
        assert_eq!(real_to_raw_voxel(500.0, ChannelDepth::Bit16), 32767);
        assert!((raw_voxel_to_real(127, ChannelDepth::Bit8) - 10.0).abs() < 1e-6);
        assert!((raw_voxel_to_real(32767, ChannelDepth::Bit16) - 500.0).abs() < 1e-3);
    }

    #[test]
    fn set_voxel_decompresses() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        vb.set_voxel(42, 0, 0, 0, ChannelId::Type.index());
        assert_eq!(
            vb.channel_compression(ChannelId::Type.index()),
            Compression::None
        );
        assert_eq!(vb.get_voxel(0, 0, 0, ChannelId::Type.index()), 42);
        // Other voxels retain the (now-materialized) default of 0.
        assert_eq!(vb.get_voxel(1, 1, 1, ChannelId::Type.index()), 0);
    }

    #[test]
    fn set_channel_depth_resets_materialized_channel_storage() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        vb.set_voxel(42, 0, 0, 0, ChannelId::Type.index());
        assert_eq!(
            vb.channel_compression(ChannelId::Type.index()),
            Compression::None
        );

        vb.set_channel_depth(ChannelId::Type.index(), ChannelDepth::Bit32);

        assert_eq!(
            vb.channel_compression(ChannelId::Type.index()),
            Compression::Uniform
        );
        assert!(vb.channel_bytes(ChannelId::Type.index()).is_empty());
        assert_eq!(
            vb.channel_depth(ChannelId::Type.index()),
            ChannelDepth::Bit32
        );
        vb.set_voxel(0x1122_3344, 1, 1, 1, ChannelId::Type.index());
        assert_eq!(vb.get_voxel(1, 1, 1, ChannelId::Type.index()), 0x1122_3344);
    }

    #[test]
    fn sdf_float_roundtrip_16bit() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        // SDF channel is 16-bit by default.
        vb.set_voxel_f(0.5, 0, 0, 0, ChannelId::Sdf.index());
        let back = vb.get_voxel_f(0, 0, 0, ChannelId::Sdf.index());
        assert!((back - 0.5).abs() < 0.02, "got {back}");
    }

    #[test]
    fn sdf_float_roundtrip_32bit() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(1, 1, 1));
        // Force 32-bit on the SDF channel.
        vb.channels[ChannelId::Sdf.index()].depth = ChannelDepth::Bit32;
        vb.set_voxel_f(1.25, 0, 0, 0, ChannelId::Sdf.index());
        assert_eq!(vb.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()), 1.25);
    }

    #[test]
    fn fill_makes_uniform() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        vb.set_voxel(1, 0, 0, 0, ChannelId::Type.index()); // decompress
        vb.fill(7, ChannelId::Type.index());
        assert_eq!(
            vb.channel_compression(ChannelId::Type.index()),
            Compression::Uniform
        );
        assert_eq!(vb.channel_default(ChannelId::Type.index()), 7);
        assert_eq!(vb.get_voxel(2, 2, 2, ChannelId::Type.index()), 7);
    }

    #[test]
    fn fill_area_writes_subregion() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(3, 3, 3));
        vb.fill_area(
            9,
            Vector3i::new(1, 1, 1),
            Vector3i::new(2, 2, 2),
            ChannelId::Type.index(),
        );
        assert_eq!(vb.get_voxel(1, 1, 1, ChannelId::Type.index()), 9);
        assert_eq!(vb.get_voxel(0, 0, 0, ChannelId::Type.index()), 0);
    }

    #[test]
    fn is_uniform_after_uniform_fill() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        vb.set_voxel(5, 0, 0, 0, ChannelId::Type.index());
        assert!(!vb.is_uniform(ChannelId::Type.index()));
        vb.fill(5, ChannelId::Type.index());
        assert!(vb.is_uniform(ChannelId::Type.index()));
    }

    #[test]
    fn compress_uniform_channels() {
        let mut vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        // Materialize then fill uniformly.
        vb.set_voxel(3, 0, 0, 0, ChannelId::Type.index());
        vb.fill(3, ChannelId::Type.index());
        // After fill it's already uniform; materialize first to test compress.
        vb.decompress_channel(ChannelId::Type.index());
        assert_eq!(
            vb.channel_compression(ChannelId::Type.index()),
            Compression::None
        );
        vb.compress_uniform_channels();
        assert_eq!(
            vb.channel_compression(ChannelId::Type.index()),
            Compression::Uniform
        );
        assert_eq!(vb.channel_default(ChannelId::Type.index()), 3);
    }

    #[test]
    fn copy_channel_from_clones() {
        let mut a = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        a.set_voxel(11, 0, 0, 0, ChannelId::Type.index());
        let mut b = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        b.copy_channel_from(&a, ChannelId::Type.index());
        assert_eq!(b.get_voxel(0, 0, 0, ChannelId::Type.index()), 11);
    }

    #[test]
    fn copy_channel_from_allocates_through_destination_pool() {
        let mut src = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        src.set_voxel(11, 0, 0, 0, ChannelId::Type.index());
        let pool = Arc::new(VoxelMemoryPool::new());
        let mut dst = VoxelBuffer::new(Allocator::Pool).with_pool(pool.clone());
        dst.create(src.size());

        dst.copy_channel_from(&src, ChannelId::Type.index());

        assert_eq!(pool.used_blocks(), 1);
        assert_eq!(dst.get_voxel(0, 0, 0, ChannelId::Type.index()), 11);

        dst.clear_channel(ChannelId::Type.index(), 0);
        assert_eq!(pool.used_blocks(), 0);
    }

    #[test]
    fn copy_channel_from_area_copies_materialized_region() {
        let channel = ChannelId::Type.index();
        let mut src = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        for z in 0..4 {
            for x in 0..4 {
                for y in 0..4 {
                    src.set_voxel((1 + y + 10 * x + 100 * z) as u64, x, y, z, channel);
                }
            }
        }
        let mut dst = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        dst.fill(999, channel);

        dst.copy_channel_from_area(
            &src,
            Vector3i::new(1, 1, 1),
            Vector3i::new(3, 3, 3),
            Vector3i::zero(),
            channel,
        );

        for z in 0..4 {
            for x in 0..4 {
                for y in 0..4 {
                    let expected = if x < 2 && y < 2 && z < 2 {
                        src.get_voxel(x + 1, y + 1, z + 1, channel)
                    } else {
                        999
                    };
                    assert_eq!(dst.get_voxel(x, y, z, channel), expected);
                }
            }
        }
    }

    #[test]
    fn copy_channel_from_area_uniform_source_overwrites_materialized_region() {
        let channel = ChannelId::Type.index();
        let src = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        let mut dst = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        dst.set_voxel(42, 1, 1, 1, channel);
        dst.set_voxel(43, 2, 2, 2, channel);

        dst.copy_channel_from_area(
            &src,
            Vector3i::zero(),
            Vector3i::new(2, 2, 2),
            Vector3i::new(1, 1, 1),
            channel,
        );

        assert_eq!(dst.get_voxel(1, 1, 1, channel), 0);
        assert_eq!(dst.get_voxel(2, 2, 2, channel), 0);
    }

    #[test]
    #[should_panic(expected = "requires equal buffer sizes")]
    fn copy_channel_from_rejects_size_mismatch() {
        let src = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        let mut dst = VoxelBuffer::with_size(Vector3i::new(1, 1, 1));
        dst.copy_channel_from(&src, ChannelId::Type.index());
    }

    #[test]
    fn depth_byte_count() {
        assert_eq!(ChannelDepth::Bit8.byte_count(), 1);
        assert_eq!(ChannelDepth::Bit16.byte_count(), 2);
        assert_eq!(ChannelDepth::Bit32.byte_count(), 4);
        assert_eq!(ChannelDepth::Bit64.byte_count(), 8);
    }

    #[test]
    fn pool_allocator_round_trip() {
        let pool = Arc::new(VoxelMemoryPool::new());
        {
            let mut vb = VoxelBuffer::new(Allocator::Pool).with_pool(pool.clone());
            vb.create(Vector3i::new(4, 4, 4));
            vb.set_voxel(1, 0, 0, 0, ChannelId::Type.index());
            assert_eq!(vb.get_voxel(0, 0, 0, ChannelId::Type.index()), 1);
            // Drop returns the allocation to the pool.
        }
        // Pool should have some memory after the drop (used_blocks back to 0).
        assert_eq!(pool.used_blocks(), 0);
    }

    #[test]
    fn create_recycles_existing_pooled_channel_data() {
        let pool = Arc::new(VoxelMemoryPool::new());
        let mut vb = VoxelBuffer::new(Allocator::Pool).with_pool(pool.clone());
        vb.create(Vector3i::new(4, 4, 4));
        vb.set_voxel(1, 0, 0, 0, ChannelId::Type.index());
        assert_eq!(pool.used_blocks(), 1);

        vb.create(Vector3i::new(2, 2, 2));

        assert_eq!(
            pool.used_blocks(),
            0,
            "create() must return materialized pooled channels before resetting"
        );
    }

    #[test]
    fn channel_id_names() {
        assert_eq!(ChannelId::Type.name(), "type");
        assert_eq!(ChannelId::Sdf.name(), "sdf");
        assert_eq!(ChannelId::Data7.name(), "data7");
    }

    #[test]
    fn real_to_raw_32bit_is_bit_cast() {
        assert_eq!(
            real_to_raw_voxel(1.5, ChannelDepth::Bit32),
            f32::to_bits(1.5) as u64
        );
        assert_eq!(
            raw_voxel_to_real(f32::to_bits(1.5) as u64, ChannelDepth::Bit32),
            1.5
        );
    }

    #[test]
    fn downscale_to_samples_nearest_neighbor_2_to_1() {
        // Build a 4×4×4 source where each voxel carries its ZXY index in the
        // Type channel, so we can verify exactly which source voxel each dst
        // cell sampled.
        let channel = ChannelId::Type.index();
        let mut src = VoxelBuffer::with_size(Vector3i::splat(4));
        for z in 0..4 {
            for x in 0..4 {
                for y in 0..4 {
                    let v = (z * 16 + x * 4 + y) as u64;
                    src.set_voxel(v, x, y, z, channel);
                }
            }
        }

        let mut dst = VoxelBuffer::with_size(Vector3i::splat(2));
        src.downscale_to(
            &mut dst,
            Vector3i::zero(),
            Vector3i::splat(4),
            Vector3i::zero(),
        );

        for z in 0..2 {
            for x in 0..2 {
                for y in 0..2 {
                    let expected = ((z * 2) * 16 + (x * 2) * 4 + (y * 2)) as u64;
                    assert_eq!(dst.get_voxel(x, y, z, channel), expected);
                }
            }
        }
    }

    #[test]
    fn downscale_to_skips_uniform_channels_with_matching_default() {
        let channel = ChannelId::Type.index();
        let mut src = VoxelBuffer::with_size(Vector3i::splat(4));
        src.fill(7, channel);
        // SDF stays at its default far-outside sentinel on both ends.

        let mut dst = VoxelBuffer::with_size(Vector3i::splat(2));
        src.downscale_to(
            &mut dst,
            Vector3i::zero(),
            Vector3i::splat(4),
            Vector3i::zero(),
        );

        // Type channel was uniform-7, dst was uniform-0 → materialized to 7.
        assert_eq!(dst.get_voxel(0, 0, 0, channel), 7);
        // SDF channel was uniform on both ends with equal defaults → untouched,
        // stays uniform (no allocation).
        assert_eq!(
            dst.channel_compression(ChannelId::Sdf.index()),
            Compression::Uniform
        );
    }

    #[test]
    fn downscale_to_clamps_oversized_source_region_into_dst_bounds() {
        // Source region extends past the source buffer; the implementation
        // clamps it to the available 4³ region before sampling. The dst min
        // stays at the origin so the whole dst buffer is filled.
        let channel = ChannelId::Type.index();
        let mut src = VoxelBuffer::with_size(Vector3i::splat(4));
        src.fill(3, channel);
        let mut dst = VoxelBuffer::with_size(Vector3i::splat(2));

        src.downscale_to(
            &mut dst,
            Vector3i::zero(),
            Vector3i::splat(99),
            Vector3i::zero(),
        );

        for z in 0..2 {
            for x in 0..2 {
                for y in 0..2 {
                    assert_eq!(dst.get_voxel(x, y, z, channel), 3);
                }
            }
        }
    }

    #[test]
    fn downscale_to_into_destination_subregion_uses_offset_mapping() {
        // Writing into a non-zero dst_min still maps back to the correct
        // source voxel via `src_min + ((dst_pos - dst_min) << 1)`.
        let channel = ChannelId::Type.index();
        let mut src = VoxelBuffer::with_size(Vector3i::splat(4));
        src.fill(5, channel);
        // Materialize a single marker voxel.
        src.set_voxel(42, 2, 0, 0, channel);

        let mut dst = VoxelBuffer::with_size(Vector3i::splat(4));
        // Downscale the 4³ source into the (1..3)³ region of an 4³ dst buffer.
        src.downscale_to(
            &mut dst,
            Vector3i::zero(),
            Vector3i::splat(4),
            Vector3i::new(1, 1, 1),
        );

        // dst(1,1,1) samples src(0,0,0) = 5; dst(2,*,*) samples src(2,*,*) so
        // dst(2,1,1) = src(2,0,0) = 42.
        assert_eq!(dst.get_voxel(1, 1, 1, channel), 5);
        assert_eq!(dst.get_voxel(2, 1, 1, channel), 42);
    }
}
