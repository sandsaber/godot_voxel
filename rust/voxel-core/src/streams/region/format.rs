//! Pure-data types for the region-file format.
//!
//! Ported from the `RegionFormat` / `RegionBlockInfo` structs in
//! `streams/region/region_file.h`. These describe the on-disk layout; the
//! actual read/write logic lives in [`super::region_file`].

use crate::math::{Color8, Vector3i};
use crate::storage::voxel_buffer::MAX_CHANNELS;
use crate::storage::{ChannelDepth, VoxelBuffer};

/// File extension for region files. Matches `RegionFormat::FILE_EXTENSION`.
pub const FILE_EXTENSION: &str = "vxr";

/// Latest on-disk version, written by [`super::region_file::RegionFile`].
pub const FORMAT_VERSION: u8 = 3;

/// ASCII magic at the start of every region file. Matches `FORMAT_REGION_MAGIC`.
pub const MAGIC: &[u8; 4] = b"VXR_";

/// Magic (4 bytes) + version (1 byte).
pub const MAGIC_AND_VERSION_SIZE: usize = 5;

/// `block_size_po2(1) + region_size(3) + sector_size(2) + palette_flag(1)` = 7,
/// plus 8 channel-depth bytes.
pub const FIXED_HEADER_DATA_SIZE: usize = 7 + MAX_CHANNELS;

/// Bytes occupied by the palette when present: 256 entries × 4 bytes.
pub const PALETTE_SIZE_IN_BYTES: usize = 256 * 4;

/// Maximum number of blocks along any region axis. Stored as one byte on disk,
/// so the practical limit is 255. Matches `RegionFormat::MAX_BLOCKS_ACROSS`.
pub const MAX_BLOCKS_ACROSS: u32 = 255;

/// 24-bit sector index cap. Matches `RegionBlockInfo::MAX_SECTOR_INDEX`.
pub const MAX_SECTOR_INDEX: u32 = 0xffffff;
/// 8-bit sector count cap. Matches `RegionBlockInfo::MAX_SECTOR_COUNT`.
pub const MAX_SECTOR_COUNT: u32 = 0xff;

/// Describes the voxel format of a region file. Ported from `RegionFormat`.
///
/// All blocks in a region share this format; it is written once in the header
/// and verified on every `save_block`.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionFormat {
    /// Voxels per cubic block, as a power of two (e.g. 4 → 16³ blocks).
    pub block_size_po2: u8,
    /// Number of blocks along each axis.
    pub region_size: Vector3i,
    /// Per-channel bit depth (fixed at 8 channels).
    pub channel_depths: [ChannelDepth; MAX_CHANNELS],
    /// Block data is stored at offsets that are multiples of this size.
    pub sector_size: u32,
    /// Optional 256-entry color palette.
    pub palette: Option<[Color8; 256]>,
}

impl Default for RegionFormat {
    fn default() -> Self {
        // Matches the C++ RegionFile() ctor defaults.
        Self {
            block_size_po2: 4, // 16³ blocks
            region_size: Vector3i::new(16, 16, 16),
            channel_depths: [ChannelDepth::Bit8; MAX_CHANNELS],
            sector_size: 512,
            palette: None,
        }
    }
}

impl RegionFormat {
    /// Whether `self` is a valid, serializable format. Ported from `validate`.
    pub fn validate(&self) -> bool {
        if self.region_size.x < 0 || self.region_size.x as u32 >= MAX_BLOCKS_ACROSS {
            return false;
        }
        if self.region_size.y < 0 || self.region_size.y as u32 >= MAX_BLOCKS_ACROSS {
            return false;
        }
        if self.region_size.z < 0 || self.region_size.z as u32 >= MAX_BLOCKS_ACROSS {
            return false;
        }
        if !(1..=8).contains(&self.block_size_po2) {
            return false;
        }

        // Worst-case: every channel fully allocated at max depth.
        let voxels_per_block = 1u64 << (3 * self.block_size_po2 as u32);
        let mut bytes_per_block = 0u64;
        for d in &self.channel_depths {
            bytes_per_block += (d.bit_count() / 8) as u64 * voxels_per_block;
        }
        if self.sector_size == 0 {
            return false;
        }
        let sectors_per_block = bytes_per_block.div_ceil(self.sector_size as u64);
        if sectors_per_block > MAX_SECTOR_COUNT as u64 {
            return false;
        }
        let max_potential_sectors = self.region_size.volume_u64() * sectors_per_block;
        if max_potential_sectors > MAX_SECTOR_INDEX as u64 {
            return false;
        }
        true
    }

    /// Whether `block` matches this region's format (size + per-channel depth).
    /// Ported from `verify_block`.
    pub fn verify_block(&self, block: &VoxelBuffer) -> bool {
        let expected_size = Vector3i::splat(1i32 << self.block_size_po2);
        if block.size() != expected_size {
            return false;
        }
        for (i, &expected_depth) in self.channel_depths.iter().enumerate() {
            if block.channel_depth(i) != expected_depth {
                return false;
            }
        }
        true
    }

    /// Byte offset where block data begins (i.e. header end). Matches
    /// `get_header_size_v3`.
    pub fn header_size_v3(&self) -> usize {
        let palette_bytes = if self.palette.is_some() {
            PALETTE_SIZE_IN_BYTES
        } else {
            0
        };
        MAGIC_AND_VERSION_SIZE
            + FIXED_HEADER_DATA_SIZE
            + palette_bytes
            + self.region_size.volume_u64() as usize * std::mem::size_of::<RegionBlockInfo>()
    }
}

/// Location and size of one block within the data area. Ported from
/// `RegionBlockInfo`.
///
/// Packed into a single `u32`: bits 31..8 = sector_index (24 bits), bits 7..0
/// = sector_count (8 bits). `data == 0` means the block slot is empty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct RegionBlockInfo {
    pub data: u32,
}

impl RegionBlockInfo {
    /// An empty/unallocated block slot.
    pub const EMPTY: Self = Self { data: 0 };

    /// Build from sector index + count.
    pub fn new(sector_index: u32, sector_count: u32) -> Self {
        debug_assert!(sector_index <= MAX_SECTOR_INDEX);
        debug_assert!(sector_count <= MAX_SECTOR_COUNT);
        Self {
            data: (sector_index << 8) | (sector_count & 0xff),
        }
    }

    /// `get_sector_index` — offset into the data area, in sectors.
    #[inline]
    pub fn sector_index(self) -> u32 {
        self.data >> 8
    }

    /// `set_sector_index`.
    #[inline]
    pub fn set_sector_index(&mut self, i: u32) {
        debug_assert!(i <= MAX_SECTOR_INDEX);
        self.data = (i << 8) | (self.data & 0xff);
    }

    /// `get_sector_count` — how many consecutive sectors the block occupies.
    #[inline]
    pub fn sector_count(self) -> u32 {
        self.data & 0xff
    }

    /// `set_sector_count`.
    #[inline]
    pub fn set_sector_count(&mut self, c: u32) {
        debug_assert!(c <= MAX_SECTOR_COUNT);
        self.data = (c & 0xff) | (self.data & 0xffffff00);
    }

    /// Whether this slot is allocated.
    #[inline]
    pub fn is_present(self) -> bool {
        self.data != 0
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn block_info_pack_unpack_round_trips() {
        let bi = RegionBlockInfo::new(0x123456, 0x78);
        assert_eq!(bi.sector_index(), 0x123456);
        assert_eq!(bi.sector_count(), 0x78);
        assert!(bi.is_present());
    }

    #[test]
    fn block_info_empty_is_zero() {
        assert_eq!(RegionBlockInfo::EMPTY.data, 0);
        assert!(!RegionBlockInfo::EMPTY.is_present());
    }

    #[test]
    fn block_info_setter_preserves_other_field() {
        let mut bi = RegionBlockInfo::new(100, 5);
        bi.set_sector_index(200);
        assert_eq!(bi.sector_index(), 200);
        assert_eq!(bi.sector_count(), 5);
        bi.set_sector_count(10);
        assert_eq!(bi.sector_index(), 200);
        assert_eq!(bi.sector_count(), 10);
    }

    #[test]
    fn default_format_is_valid() {
        assert!(RegionFormat::default().validate());
    }

    #[test]
    fn format_rejects_zero_block_size_po2() {
        let mut f = RegionFormat::default();
        f.block_size_po2 = 0;
        assert!(!f.validate());
    }

    #[test]
    fn format_rejects_oversized_region() {
        let mut f = RegionFormat::default();
        f.region_size = Vector3i::new(255, 16, 16); // >= MAX_BLOCKS_ACROSS
        assert!(!f.validate());
    }

    #[test]
    fn format_rejects_negative_region_axis() {
        let mut f = RegionFormat::default();
        f.region_size = Vector3i::new(-1, 16, 16);
        assert!(!f.validate());
    }

    #[test]
    fn header_size_v3_matches_expected_layout() {
        let f = RegionFormat {
            block_size_po2: 4,
            region_size: Vector3i::new(16, 16, 16),
            channel_depths: [ChannelDepth::Bit8; MAX_CHANNELS],
            sector_size: 512,
            palette: None,
        };
        // magic(4) + version(1) + block_size_po2(1) + region_size(3) +
        // channel_depths(8) + sector_size(2) + palette_flag(1) + LUT(16³×4)
        let lut = 16 * 16 * 16 * 4;
        assert_eq!(f.header_size_v3(), 5 + 1 + 3 + 8 + 2 + 1 + lut);
    }

    #[test]
    fn header_size_includes_palette_when_present() {
        let mut f = RegionFormat::default();
        let without = f.header_size_v3();
        f.palette = Some([Color8::new(0, 0, 0, 0); 256]);
        let with = f.header_size_v3();
        assert_eq!(with - without, PALETTE_SIZE_IN_BYTES);
    }
}
