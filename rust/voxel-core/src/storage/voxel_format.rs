//! Per-channel depth descriptor + defaults.
//!
//! Ported from `storage/voxel_format.{h,cpp}`. Describes the bit-depth of each
//! of a [`VoxelBuffer`]'s channels and the constraints on which depths each
//! channel may use. Used to round-trip a buffer's "format" across save/load and
//! to reconfigure a buffer in place.

use super::depth::ChannelDepth;
use super::voxel_buffer::{
    get_default_raw_value, ChannelId, VoxelBuffer, DEFAULT_CHANNEL_DEPTH,
    DEFAULT_INDICES_CHANNEL_DEPTH, DEFAULT_SDF_CHANNEL_DEPTH, DEFAULT_TYPE_CHANNEL_DEPTH,
    DEFAULT_WEIGHTS_CHANNEL_DEPTH, MAX_CHANNELS,
};
const CHANNEL_IDS: [ChannelId; MAX_CHANNELS] = [
    ChannelId::Type,
    ChannelId::Sdf,
    ChannelId::Color,
    ChannelId::Indices,
    ChannelId::Weights,
    ChannelId::Data5,
    ChannelId::Data6,
    ChannelId::Data7,
];

/// A min/max range of supported [`ChannelDepth`]s for a given channel.
/// Matches `VoxelFormat::DepthRange` (C++ stores raw `uint32_t` depth indices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthRange {
    pub min: ChannelDepth,
    pub max: ChannelDepth,
}

impl DepthRange {
    #[inline]
    pub fn contains(self, depth: ChannelDepth) -> bool {
        (depth as u8) >= (self.min as u8) && (depth as u8) <= (self.max as u8)
    }
}

/// Per-channel depth configuration. Matches `VoxelFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoxelFormat {
    /// One depth per channel, indexed by [`ChannelId`] discriminant.
    pub depths: [ChannelDepth; MAX_CHANNELS],
}

impl Default for VoxelFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl VoxelFormat {
    /// Construct with the engine's default depths. Matches the C++ ctor.
    pub fn new() -> Self {
        let mut depths = [DEFAULT_CHANNEL_DEPTH; MAX_CHANNELS];
        depths[ChannelId::Type.index()] = DEFAULT_TYPE_CHANNEL_DEPTH;
        depths[ChannelId::Sdf.index()] = DEFAULT_SDF_CHANNEL_DEPTH;
        depths[ChannelId::Indices.index()] = DEFAULT_INDICES_CHANNEL_DEPTH;
        depths[ChannelId::Weights.index()] = DEFAULT_WEIGHTS_CHANNEL_DEPTH;
        // Color / Data5..7 stay at DEFAULT_CHANNEL_DEPTH (8-bit).
        Self { depths }
    }

    /// The default raw value for `channel` at this format's depth for it.
    /// Matches `get_default_raw_value`.
    #[inline]
    pub fn default_raw_value(&self, channel: ChannelId) -> u64 {
        get_default_raw_value(channel, self.depths[channel.index()])
    }

    /// Which depths a channel may use. Matches `get_supported_depths`.
    pub fn supported_depths(channel: ChannelId) -> DepthRange {
        use ChannelDepth::*;
        match channel {
            // { 1, 2 } bytes → 8 or 16 bit.
            ChannelId::Type | ChannelId::Indices => DepthRange {
                min: Bit8,
                max: Bit16,
            },
            // { 1, 4 } bytes → 8, 16, 32 bit (the C++ max index 3 == 64-bit, but
            // the byte count 4 maps to DEPTH_32_BIT; the range is 1..=4 bytes).
            ChannelId::Sdf
            | ChannelId::Color
            | ChannelId::Data5
            | ChannelId::Data6
            | ChannelId::Data7 => DepthRange {
                min: Bit8,
                max: Bit32,
            },
            // { 2, 2 } bytes → 16 bit only.
            ChannelId::Weights => DepthRange {
                min: Bit16,
                max: Bit16,
            },
        }
    }

    /// Reconfigure `vb` to this format, preserving its size. Matches
    /// `configure_buffer`. Clears the buffer.
    pub fn configure_buffer(&self, vb: &mut VoxelBuffer) {
        let size = vb.size();
        vb.create(size);
        apply_format_depths(vb, self);
    }
}

/// Apply this format's depths to `vb`'s channels (without reallocating).
fn apply_format_depths(vb: &mut VoxelBuffer, format: &VoxelFormat) {
    for (ci, channel) in CHANNEL_IDS.iter().copied().enumerate() {
        let depth = format.depths[ci];
        vb.set_channel_depth(ci, depth);
        vb.clear_channel(ci, get_default_raw_value(channel, depth));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vector3i;
    use ChannelDepth::*;

    #[test]
    fn default_format_depths() {
        let f = VoxelFormat::new();
        assert_eq!(f.depths[ChannelId::Type.index()], Bit16);
        assert_eq!(f.depths[ChannelId::Sdf.index()], Bit16);
        assert_eq!(f.depths[ChannelId::Color.index()], Bit8);
        assert_eq!(f.depths[ChannelId::Indices.index()], Bit16);
        assert_eq!(f.depths[ChannelId::Weights.index()], Bit16);
        assert_eq!(f.depths[ChannelId::Data5.index()], Bit8);
    }

    #[test]
    fn supported_depths_ranges() {
        assert_eq!(
            VoxelFormat::supported_depths(ChannelId::Type),
            DepthRange {
                min: Bit8,
                max: Bit16
            }
        );
        assert_eq!(
            VoxelFormat::supported_depths(ChannelId::Sdf),
            DepthRange {
                min: Bit8,
                max: Bit32
            }
        );
        assert_eq!(
            VoxelFormat::supported_depths(ChannelId::Weights),
            DepthRange {
                min: Bit16,
                max: Bit16
            }
        );
    }

    #[test]
    fn depth_range_contains() {
        let r = DepthRange {
            min: Bit8,
            max: Bit16,
        };
        assert!(r.contains(Bit8));
        assert!(r.contains(Bit16));
        assert!(!r.contains(Bit32));
    }

    #[test]
    fn default_raw_value_uses_format_depth() {
        let f = VoxelFormat::new();
        // SDF at 16-bit default = max positive snorm, i.e. far outside/air.
        assert_eq!(f.default_raw_value(ChannelId::Sdf), i16::MAX as u16 as u64);
        // Type at 16-bit default = 0 (air).
        assert_eq!(f.default_raw_value(ChannelId::Type), 0);
    }

    #[test]
    fn configure_buffer_applies_custom_depths_and_defaults() {
        let mut f = VoxelFormat::new();
        f.depths[ChannelId::Sdf.index()] = Bit32;
        f.depths[ChannelId::Color.index()] = Bit16;

        let mut vb = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        vb.set_voxel_f(-3.0, 0, 0, 0, ChannelId::Sdf.index());

        f.configure_buffer(&mut vb);

        assert_eq!(vb.size(), Vector3i::new(2, 2, 2));
        assert_eq!(vb.channel_depth(ChannelId::Sdf.index()), Bit32);
        assert_eq!(vb.channel_depth(ChannelId::Color.index()), Bit16);
        assert_eq!(
            vb.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()),
            crate::storage::voxel_buffer::SDF_FAR_OUTSIDE
        );
    }

    #[test]
    fn format_equality() {
        assert_eq!(VoxelFormat::new(), VoxelFormat::new());
        let mut f = VoxelFormat::new();
        f.depths[ChannelId::Color.index()] = Bit16;
        assert_ne!(f, VoxelFormat::new());
    }
}
