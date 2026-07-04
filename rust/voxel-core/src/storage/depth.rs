//! Channel depth enum, ported from `VoxelBuffer::Depth` in `storage/voxel_buffer.h`.
//!
//! Used by meshers to pick the right template instantiation per-channel.

/// Bit depth of a single voxel channel. Matches C++ `VoxelBuffer::Depth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChannelDepth {
    /// 8-bit per voxel.
    Bit8 = 0,
    /// 16-bit per voxel (default for SDF / indices / weights).
    Bit16 = 1,
    /// 32-bit per voxel (e.g. `f32` SDF).
    Bit32 = 2,
    /// 64-bit per voxel.
    Bit64 = 3,
}

impl ChannelDepth {
    /// Bytes per voxel for this depth.
    #[inline]
    pub const fn byte_size(self) -> usize {
        match self {
            ChannelDepth::Bit8 => 1,
            ChannelDepth::Bit16 => 2,
            ChannelDepth::Bit32 => 4,
            ChannelDepth::Bit64 => 8,
        }
    }

    /// Matches `VoxelBuffer::get_depth_from_size(size_t)`.
    pub fn from_byte_size(size: usize) -> Option<Self> {
        match size {
            1 => Some(ChannelDepth::Bit8),
            2 => Some(ChannelDepth::Bit16),
            4 => Some(ChannelDepth::Bit32),
            8 => Some(ChannelDepth::Bit64),
            _ => None,
        }
    }

    /// Default depth for the SDF channel (`DEFAULT_SDF_CHANNEL_DEPTH`).
    pub const DEFAULT_SDF: ChannelDepth = ChannelDepth::Bit16;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_size_roundtrip() {
        for d in [
            ChannelDepth::Bit8,
            ChannelDepth::Bit16,
            ChannelDepth::Bit32,
            ChannelDepth::Bit64,
        ] {
            assert_eq!(ChannelDepth::from_byte_size(d.byte_size()), Some(d));
        }
        assert_eq!(ChannelDepth::from_byte_size(3), None);
    }
}
