//! Sparse map of voxel data blocks for one LOD.

use crate::constants::voxel_constants::{DEFAULT_BLOCK_SIZE_PO2, MAX_LOD};
use crate::math::{Box3i, Vector3i};
use crate::storage::{
    voxel_buffer::SDF_FAR_OUTSIDE, ChannelId, VoxelBuffer, VoxelDataBlock, VoxelFormat,
};
use std::collections::HashMap;

#[derive(Debug)]
pub struct VoxelDataMap {
    blocks: HashMap<Vector3i, VoxelDataBlock>,
    lod_index: u8,
    format: VoxelFormat,
}

impl VoxelDataMap {
    pub const BLOCK_SIZE_PO2: u8 = DEFAULT_BLOCK_SIZE_PO2;
    pub const BLOCK_SIZE: u32 = 1 << Self::BLOCK_SIZE_PO2;
    pub const BLOCK_SIZE_MASK: u32 = Self::BLOCK_SIZE - 1;

    pub fn new(lod_index: u8) -> Self {
        assert!(
            usize::from(lod_index) < MAX_LOD,
            "LOD index is outside the supported range"
        );
        Self {
            blocks: HashMap::new(),
            lod_index,
            format: VoxelFormat::new(),
        }
    }

    pub fn create(&mut self, lod_index: u8) {
        assert!(
            usize::from(lod_index) < MAX_LOD,
            "LOD index is outside the supported range"
        );
        self.clear();
        self.lod_index = lod_index;
    }

    pub const fn lod_index(&self) -> u8 {
        self.lod_index
    }

    pub const fn block_size(&self) -> u32 {
        Self::BLOCK_SIZE
    }

    pub const fn block_size_pow2(&self) -> u8 {
        Self::BLOCK_SIZE_PO2
    }

    pub const fn block_size_mask(&self) -> u32 {
        Self::BLOCK_SIZE_MASK
    }

    pub fn set_format(&mut self, format: VoxelFormat) {
        self.format = format;
    }

    pub const fn format(&self) -> &VoxelFormat {
        &self.format
    }

    pub fn voxel_to_block_b(pos: Vector3i, block_size_pow2: u8) -> Vector3i {
        pos >> u32::from(block_size_pow2)
    }

    pub fn voxel_to_block(&self, pos: Vector3i) -> Vector3i {
        Self::voxel_to_block_b(pos, Self::BLOCK_SIZE_PO2)
    }

    pub fn to_local(&self, pos: Vector3i) -> Vector3i {
        pos & Self::BLOCK_SIZE_MASK
    }

    pub fn block_to_voxel(&self, block_pos: Vector3i) -> Vector3i {
        block_pos * Self::BLOCK_SIZE as i32
    }

    pub fn get_voxel(&self, pos: Vector3i, channel_index: usize) -> u64 {
        let block_pos = self.voxel_to_block(pos);
        let Some(block) = self.get_block(block_pos) else {
            return self.default_raw_value(channel_index);
        };
        if !block.has_voxels() {
            return self.default_raw_value(channel_index);
        }
        let local_pos = self.to_local(pos);
        block
            .voxels()
            .get_voxel(local_pos.x, local_pos.y, local_pos.z, channel_index)
    }

    pub fn set_voxel(&mut self, value: u64, pos: Vector3i, channel_index: usize) {
        let local_pos = self.to_local(pos);
        let block = self.get_or_create_block_at_voxel_pos(pos);
        block
            .voxels_mut()
            .set_voxel(value, local_pos.x, local_pos.y, local_pos.z, channel_index);
    }

    pub fn get_voxel_f(&self, pos: Vector3i, channel_index: usize) -> f32 {
        let block_pos = self.voxel_to_block(pos);
        let Some(block) = self.get_block(block_pos) else {
            return SDF_FAR_OUTSIDE;
        };
        if !block.has_voxels() {
            return SDF_FAR_OUTSIDE;
        }
        let local_pos = self.to_local(pos);
        block
            .voxels()
            .get_voxel_f(local_pos.x, local_pos.y, local_pos.z, channel_index)
    }

    pub fn set_voxel_f(&mut self, value: f32, pos: Vector3i, channel_index: usize) {
        let local_pos = self.to_local(pos);
        let block = self.get_or_create_block_at_voxel_pos(pos);
        block
            .voxels_mut()
            .set_voxel_f(value, local_pos.x, local_pos.y, local_pos.z, channel_index);
    }

    pub fn set_block_buffer(
        &mut self,
        block_pos: Vector3i,
        voxels: VoxelBuffer,
        overwrite: bool,
    ) -> &mut VoxelDataBlock {
        if !self.blocks.contains_key(&block_pos) {
            self.blocks.insert(
                block_pos,
                VoxelDataBlock::with_voxels(voxels, self.lod_index),
            );
        } else if overwrite {
            self.blocks
                .get_mut(&block_pos)
                .expect("block existence was checked")
                .set_voxels(voxels);
        }
        self.blocks
            .get_mut(&block_pos)
            .expect("block exists after set_block_buffer")
    }

    pub fn set_empty_block(&mut self, block_pos: Vector3i, overwrite: bool) -> &mut VoxelDataBlock {
        if !self.blocks.contains_key(&block_pos) {
            self.blocks
                .insert(block_pos, VoxelDataBlock::empty(self.lod_index));
        } else if overwrite {
            self.blocks
                .get_mut(&block_pos)
                .expect("block existence was checked")
                .clear_voxels();
        }
        self.blocks
            .get_mut(&block_pos)
            .expect("block exists after set_empty_block")
    }

    pub fn remove_block(&mut self, block_pos: Vector3i) -> Option<VoxelDataBlock> {
        self.blocks.remove(&block_pos)
    }

    pub fn get_block(&self, block_pos: Vector3i) -> Option<&VoxelDataBlock> {
        self.blocks.get(&block_pos)
    }

    pub fn get_block_mut(&mut self, block_pos: Vector3i) -> Option<&mut VoxelDataBlock> {
        self.blocks.get_mut(&block_pos)
    }

    pub fn has_block(&self, block_pos: Vector3i) -> bool {
        self.blocks.contains_key(&block_pos)
    }

    pub fn is_block_surrounded(&self, block_pos: Vector3i) -> bool {
        crate::constants::cube_tables::MOORE_NEIGHBORING_3D
            .iter()
            .all(|offset| self.has_block(block_pos + *offset))
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_area_fully_loaded(&self, voxels_box: Box3i) -> bool {
        let block_box = voxels_box.downscaled(Self::BLOCK_SIZE as i32);
        block_box.all_cells_match(|pos| self.has_block(pos))
    }

    fn get_or_create_block_at_voxel_pos(&mut self, pos: Vector3i) -> &mut VoxelDataBlock {
        let block_pos = self.voxel_to_block(pos);
        if !self.blocks.contains_key(&block_pos) {
            let block = self.create_default_block(block_pos);
            return block;
        }
        self.blocks
            .get_mut(&block_pos)
            .expect("block existence was checked")
    }

    fn create_default_block(&mut self, block_pos: Vector3i) -> &mut VoxelDataBlock {
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(Self::BLOCK_SIZE as i32));
        self.format.configure_buffer(&mut voxels);
        self.blocks.insert(
            block_pos,
            VoxelDataBlock::with_voxels(voxels, self.lod_index),
        );
        self.blocks
            .get_mut(&block_pos)
            .expect("block exists after create_default_block")
    }

    fn default_raw_value(&self, channel_index: usize) -> u64 {
        self.format
            .default_raw_value(channel_id_from_index(channel_index))
    }
}

impl Default for VoxelDataMap {
    fn default() -> Self {
        Self::new(0)
    }
}

fn channel_id_from_index(channel_index: usize) -> ChannelId {
    match channel_index {
        0 => ChannelId::Type,
        1 => ChannelId::Sdf,
        2 => ChannelId::Color,
        3 => ChannelId::Indices,
        4 => ChannelId::Weights,
        5 => ChannelId::Data5,
        6 => ChannelId::Data6,
        7 => ChannelId::Data7,
        _ => panic!("channel index is outside the supported range"),
    }
}

#[cfg(test)]
mod tests {
    use super::VoxelDataMap;
    use crate::math::{Box3i, Vector3i};
    use crate::storage::{
        voxel_buffer::SDF_FAR_OUTSIDE, ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat,
    };

    #[test]
    fn block_coordinate_conversions_match_cpp_negative_arithmetic_shift() {
        assert_eq!(
            VoxelDataMap::voxel_to_block_b(Vector3i::new(-1, -16, -17), 4),
            Vector3i::new(-1, -1, -2)
        );

        let map = VoxelDataMap::new(0);
        assert_eq!(
            map.voxel_to_block(Vector3i::new(16, 0, -1)),
            Vector3i::new(1, 0, -1)
        );
        assert_eq!(
            map.to_local(Vector3i::new(-1, -16, -17)),
            Vector3i::new(15, 0, 15)
        );
        assert_eq!(
            map.block_to_voxel(Vector3i::new(-2, 0, 3)),
            Vector3i::new(-32, 0, 48)
        );
    }

    #[test]
    fn set_and_get_voxels_create_formatted_blocks() {
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        let mut map = VoxelDataMap::new(2);
        map.set_format(format);

        let pos = Vector3i::new(-1, 0, 17);
        map.set_voxel(42, pos, ChannelId::Type.index());
        map.set_voxel_f(-12.5, pos, ChannelId::Sdf.index());

        assert_eq!(map.lod_index(), 2);
        assert_eq!(map.block_count(), 1);
        assert_eq!(map.get_voxel(pos, ChannelId::Type.index()), 42);
        assert_eq!(map.get_voxel_f(pos, ChannelId::Sdf.index()), -12.5);

        let block = map.get_block(Vector3i::new(-1, 0, 1)).unwrap();
        assert_eq!(
            block.voxels().channel_depth(ChannelId::Sdf.index()),
            ChannelDepth::Bit32
        );
    }

    #[test]
    fn missing_or_empty_blocks_return_defaults() {
        let mut map = VoxelDataMap::new(0);
        let position = Vector3i::new(4, 5, 6);

        assert_eq!(map.get_voxel(position, ChannelId::Type.index()), 0);
        assert_eq!(
            map.get_voxel(position, ChannelId::Sdf.index()),
            VoxelFormat::new().default_raw_value(ChannelId::Sdf)
        );
        assert_eq!(
            map.get_voxel_f(position, ChannelId::Sdf.index()),
            SDF_FAR_OUTSIDE
        );

        map.set_empty_block(Vector3i::zero(), true);

        assert_eq!(map.get_voxel(position, ChannelId::Type.index()), 0);
        assert_eq!(
            map.get_voxel_f(position, ChannelId::Sdf.index()),
            SDF_FAR_OUTSIDE
        );
    }

    #[test]
    fn block_insert_overwrite_and_removal_match_cpp_contract() {
        let mut map = VoxelDataMap::new(0);
        let block_pos = Vector3i::new(1, 2, 3);
        let mut first = VoxelBuffer::with_size(Vector3i::splat(VoxelDataMap::BLOCK_SIZE as i32));
        first.set_voxel(1, 0, 0, 0, ChannelId::Type.index());
        let mut second = VoxelBuffer::with_size(Vector3i::splat(VoxelDataMap::BLOCK_SIZE as i32));
        second.set_voxel(2, 0, 0, 0, ChannelId::Type.index());

        map.set_block_buffer(block_pos, first, false);
        map.set_block_buffer(block_pos, second, false);

        assert_eq!(map.block_count(), 1);
        assert_eq!(
            map.get_block(block_pos)
                .unwrap()
                .voxels()
                .get_voxel(0, 0, 0, ChannelId::Type.index()),
            1
        );

        let mut replacement =
            VoxelBuffer::with_size(Vector3i::splat(VoxelDataMap::BLOCK_SIZE as i32));
        replacement.set_voxel(3, 0, 0, 0, ChannelId::Type.index());
        map.set_block_buffer(block_pos, replacement, true);

        assert_eq!(
            map.get_block(block_pos)
                .unwrap()
                .voxels()
                .get_voxel(0, 0, 0, ChannelId::Type.index()),
            3
        );

        map.set_empty_block(block_pos, true);
        assert!(!map.get_block(block_pos).unwrap().has_voxels());

        assert!(map.remove_block(block_pos).is_some());
        assert!(!map.has_block(block_pos));
        assert_eq!(map.block_count(), 0);
    }

    #[test]
    fn area_loaded_requires_every_overlapped_block() {
        let mut map = VoxelDataMap::new(0);
        let area = Box3i::new(Vector3i::new(8, 0, 0), Vector3i::new(24, 16, 16));

        assert!(!map.is_area_fully_loaded(area));

        map.set_empty_block(Vector3i::new(0, 0, 0), true);
        assert!(!map.is_area_fully_loaded(area));

        map.set_empty_block(Vector3i::new(1, 0, 0), true);
        assert!(map.is_area_fully_loaded(area));
    }
}
