//! Voxel buffer types.
//!
//! Ported (minimally) from `storage/voxel_buffer.h`. The pilot only needs
//! read access to a channel's raw bytes plus its depth and the buffer size;
//! that contract is captured by [`VoxelBufferRead`]. The full `VoxelBuffer`
//! with compression, multiple allocators and format round-tripping arrives
//! in Phase 3.
//!
//! [`DenseVoxelBuffer`] is a simple owned implementation used for tests and
//! benchmarks in Phase 0. It mirrors the uncompressed, default-allocator path
//! of the C++ class.

use super::depth::ChannelDepth;
use crate::math::Vector3i;

/// Read-only view of a single voxel channel: raw bytes + depth + size.
///
/// This is the contract that meshers depend on. In C++ it is implemented by
/// `VoxelBuffer::get_channel_as_bytes_read_only()` together with
/// `get_channel_depth()` and `get_size()`.
pub trait VoxelBufferRead {
    /// 3D size of the buffer (number of voxels along each axis).
    fn size(&self) -> Vector3i;

    /// Depth (bit width) of the channel at `channel_index`.
    ///
    /// Implementations may panic if the channel is not present.
    fn channel_depth(&self, channel_index: u32) -> ChannelDepth;

    /// Returns the raw bytes of the channel, laid out as `size.x * size.y * size.z`
    /// voxels each of `channel_depth().byte_size()` bytes, in X-minor / Z-major
    /// order (matching the C++ `get_index(x,y,z) = x + sx*(y + sy*z)`).
    fn channel_bytes(&self, channel_index: u32) -> &[u8];
}

/// Indexing helper matching C++ `VoxelBuffer::get_index`. The C++ VoxelBuffer
/// uses a ZXY memory layout: `index = y + size.y * (x + size.x * z)`. Y is the
/// innermost axis.
#[inline]
pub fn voxel_index(size: Vector3i, x: usize, y: usize, z: usize) -> usize {
    debug_assert!(x < size.x as usize && y < size.y as usize && z < size.z as usize);
    y + (size.y as usize) * (x + (size.x as usize) * z)
}

/// Number of voxels in a buffer of the given size, as `u64`. Matches
/// `Vector3iUtil::get_volume_u64` in C++.
#[inline]
pub fn volume_u64(size: Vector3i) -> u64 {
    size.x as u64 * size.y as u64 * size.z as u64
}

// ---------------------------------------------------------------------------
// DenseVoxelBuffer — simple owned implementation for the pilot.
// ---------------------------------------------------------------------------

/// A simple dense, single-channel voxel buffer.
///
/// Owned storage, no compression. Suitable for Phase 0 tests/benchmarks where
/// we feed the mesher a plain SDF volume. The full `VoxelBuffer` (multi-channel,
/// compressed, format-aware) is migrated in Phase 3.
#[derive(Debug, Clone)]
pub struct DenseVoxelBuffer {
    size: Vector3i,
    depth: ChannelDepth,
    // Raw bytes for a single channel. Length == volume * depth.byte_size().
    data: Vec<u8>,
}

impl DenseVoxelBuffer {
    /// Create a zero-initialized buffer of `size` voxels with the given depth.
    pub fn new(size: Vector3i, depth: ChannelDepth) -> Self {
        let vol = volume_u64(size) as usize;
        let len = vol
            .checked_mul(depth.byte_size())
            .expect("buffer too large");
        Self {
            size,
            depth,
            data: vec![0; len],
        }
    }

    /// Create from a pre-filled byte vector. The length must match
    /// `volume * depth.byte_size()`.
    pub fn from_bytes(size: Vector3i, depth: ChannelDepth, data: Vec<u8>) -> Self {
        let expected = volume_u64(size) as usize * depth.byte_size();
        assert_eq!(
            data.len(),
            expected,
            "DenseVoxelBuffer data length mismatch"
        );
        Self { size, depth, data }
    }

    #[inline]
    pub fn depth(&self) -> ChannelDepth {
        self.depth
    }

    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    // -- typed element access (pilot convenience) --

    /// Read one voxel as `f32`, interpreting the channel according to its depth.
    /// 16-bit SDF values are interpreted as signed normalized in the same way
    /// the C++ code reads `int16_t` channels for transvoxel.
    pub fn get_voxel_f(&self, x: usize, y: usize, z: usize) -> f32 {
        let i = voxel_index(self.size, x, y, z) * self.depth.byte_size();
        let bytes = &self.data[i..i + self.depth.byte_size()];
        match self.depth {
            ChannelDepth::Bit8 => bytes[0] as i8 as f32,
            ChannelDepth::Bit16 => {
                let v = i16::from_le_bytes([bytes[0], bytes[1]]);
                v as f32
            }
            ChannelDepth::Bit32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            ChannelDepth::Bit64 => f64::from_le_bytes(bytes.try_into().unwrap()) as f32,
        }
    }

    /// Write one voxel from `f32`, rounding to the channel depth.
    pub fn set_voxel_f(&mut self, x: usize, y: usize, z: usize, value: f32) {
        let i = voxel_index(self.size, x, y, z) * self.depth.byte_size();
        let bytes = &mut self.data[i..i + self.depth.byte_size()];
        match self.depth {
            ChannelDepth::Bit8 => bytes[0] = value.round() as i8 as u8,
            ChannelDepth::Bit16 => {
                let v = value as i16;
                bytes[..2].copy_from_slice(&v.to_le_bytes());
            }
            ChannelDepth::Bit32 => {
                bytes[..4].copy_from_slice(&value.to_le_bytes());
            }
            ChannelDepth::Bit64 => {
                bytes[..8].copy_from_slice(&(value as f64).to_le_bytes());
            }
        }
    }
}

// For Phase 0 we only ever use channel index 0; the trait still models the
// channel-indexed C++ API so future swap-in of the real `VoxelBuffer` is a
// drop-in change.
impl VoxelBufferRead for DenseVoxelBuffer {
    #[inline]
    fn size(&self) -> Vector3i {
        self.size
    }

    #[inline]
    fn channel_depth(&self, _channel_index: u32) -> ChannelDepth {
        self.depth
    }

    #[inline]
    fn channel_bytes(&self, _channel_index: u32) -> &[u8] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vector3i;

    #[test]
    fn index_layout_is_zxy() {
        // Matches C++ VoxelBuffer::get_index(x,y,z) = y + sy*(x + sx*z).
        // Y is innermost.
        let s = Vector3i::new(4, 5, 6);
        assert_eq!(voxel_index(s, 0, 0, 0), 0);
        // Y+1 advances by 1 (Y innermost).
        assert_eq!(voxel_index(s, 0, 1, 0), 1);
        // X+1 advances by sy (=5).
        assert_eq!(voxel_index(s, 1, 0, 0), 5);
        // Z+1 advances by sy*sx (=5*4=20).
        assert_eq!(voxel_index(s, 0, 0, 1), 20);
        // Last voxel.
        assert_eq!(voxel_index(s, 3, 4, 5), 4 * 5 * 6 - 1);
    }

    #[test]
    fn roundtrip_i16_sdf() {
        let mut buf = DenseVoxelBuffer::new(Vector3i::new(2, 2, 2), ChannelDepth::Bit16);
        buf.set_voxel_f(0, 0, 0, -100.0);
        buf.set_voxel_f(1, 1, 1, 32000.0);
        assert_eq!(buf.get_voxel_f(0, 0, 0), -100.0);
        assert_eq!(buf.get_voxel_f(1, 1, 1), 32000.0);
    }

    #[test]
    fn roundtrip_f32() {
        let mut buf = DenseVoxelBuffer::new(Vector3i::new(1, 1, 1), ChannelDepth::Bit32);
        buf.set_voxel_f(0, 0, 0, 1.5);
        assert_eq!(buf.get_voxel_f(0, 0, 0), 1.5);
    }

    #[test]
    fn volume() {
        assert_eq!(volume_u64(Vector3i::new(16, 16, 16)), 4096);
    }

    #[test]
    fn channel_bytes_length_matches() {
        let buf = DenseVoxelBuffer::new(Vector3i::new(3, 4, 5), ChannelDepth::Bit16);
        // trait dispatch
        let r: &dyn VoxelBufferRead = &buf;
        assert_eq!(r.channel_bytes(0).len(), 3 * 4 * 5 * 2);
    }
}
