//! Aggregate voxel storage over LOD maps.
//!
//! First-pass, engine-agnostic port of `storage/voxel_data.{h,cpp}`. This file
//! intentionally starts with the synchronous storage contract: LOD maps, format,
//! bounds, block insertion, direct voxel edits and modification flags. Generator
//! and stream task integration are layered on top in later Phase 4 steps.

use crate::constants::voxel_constants::MAX_LOD;
use crate::math::{Box3i, Vector3i};
use crate::storage::{
    voxel_buffer::{raw_voxel_to_real, real_to_raw_voxel, SDF_FAR_OUTSIDE},
    VoxelBuffer, VoxelDataBlock, VoxelDataMap, VoxelFormat,
};

#[derive(Debug)]
struct VoxelDataLod {
    map: VoxelDataMap,
}

impl VoxelDataLod {
    fn new(lod_index: u8, format: VoxelFormat) -> Self {
        let mut map = VoxelDataMap::new(lod_index);
        map.set_format(format);
        Self { map }
    }
}

#[derive(Debug)]
pub struct VoxelData {
    lods: Vec<VoxelDataLod>,
    format: VoxelFormat,
    bounds_in_voxels: Box3i,
    full_load_completed: bool,
    streaming_enabled: bool,
}

impl Default for VoxelData {
    fn default() -> Self {
        Self::new()
    }
}

impl VoxelData {
    pub fn new() -> Self {
        let format = VoxelFormat::new();
        Self {
            lods: vec![VoxelDataLod::new(0, format)],
            format,
            bounds_in_voxels: Box3i::default(),
            full_load_completed: false,
            streaming_enabled: true,
        }
    }

    pub const fn block_size(&self) -> u32 {
        VoxelDataMap::BLOCK_SIZE
    }

    pub const fn block_size_po2(&self) -> u8 {
        VoxelDataMap::BLOCK_SIZE_PO2
    }

    pub fn voxel_to_block(&self, pos: Vector3i) -> Vector3i {
        VoxelDataMap::voxel_to_block_b(pos, self.block_size_po2())
    }

    pub fn block_to_voxel(&self, pos: Vector3i) -> Vector3i {
        pos * self.block_size() as i32
    }

    pub fn lod_count(&self) -> usize {
        self.lods.len()
    }

    pub fn set_lod_count(&mut self, lod_count: usize) {
        assert!(
            (1..MAX_LOD).contains(&lod_count),
            "LOD count is outside the supported range"
        );
        if lod_count == self.lods.len() {
            return;
        }
        self.lods = (0..lod_count)
            .map(|lod_index| VoxelDataLod::new(lod_index as u8, self.format))
            .collect();
    }

    pub fn reset_maps(&mut self) {
        for (lod_index, lod) in self.lods.iter_mut().enumerate() {
            lod.map.create(lod_index as u8);
            lod.map.set_format(self.format);
        }
    }

    pub const fn bounds(&self) -> Box3i {
        self.bounds_in_voxels
    }

    pub const fn set_bounds(&mut self, bounds: Box3i) {
        self.bounds_in_voxels = bounds;
    }

    pub const fn format(&self) -> VoxelFormat {
        self.format
    }

    pub fn set_format(&mut self, format: VoxelFormat) {
        if self.format == format {
            return;
        }
        self.format = format;
        self.reset_maps();
    }

    pub const fn is_streaming_enabled(&self) -> bool {
        self.streaming_enabled
    }

    pub const fn set_streaming_enabled(&mut self, enabled: bool) {
        self.streaming_enabled = enabled;
    }

    pub const fn is_full_load_completed(&self) -> bool {
        self.full_load_completed
    }

    pub const fn set_full_load_completed(&mut self, complete: bool) {
        self.full_load_completed = complete;
    }

    pub fn get_voxel(&self, pos: Vector3i, channel_index: usize, defval: u64) -> u64 {
        if !self.bounds_in_voxels.contains_point(pos) {
            return defval;
        }
        if !self.streaming_enabled && !self.full_load_completed {
            return defval;
        }
        let block_pos = self.voxel_to_block(pos);
        let Some(block) = self.lods[0].map.get_block(block_pos) else {
            return defval;
        };
        if !block.has_voxels() {
            return defval;
        }
        let local_pos = self.lods[0].map.to_local(pos);
        block
            .voxels()
            .get_voxel(local_pos.x, local_pos.y, local_pos.z, channel_index)
    }

    pub fn get_voxel_f(&self, pos: Vector3i, channel_index: usize) -> f32 {
        let raw = self.get_voxel(
            pos,
            channel_index,
            real_to_raw_voxel(SDF_FAR_OUTSIDE, self.format.depths[channel_index]),
        );
        raw_voxel_to_real(raw, self.format.depths[channel_index])
    }

    pub fn try_set_voxel(&mut self, value: u64, pos: Vector3i, channel_index: usize) -> bool {
        if !self.bounds_in_voxels.contains_point(pos) {
            return false;
        }
        let block_pos = self.voxel_to_block(pos);
        let block_state = self.lods[0]
            .map
            .get_block(block_pos)
            .map(|block| block.has_voxels());

        match block_state {
            Some(true) => {}
            Some(false) => {
                let voxels = self.create_block_buffer();
                self.lods[0].map.set_block_buffer(block_pos, voxels, true);
            }
            None => {
                if self.streaming_enabled || !self.full_load_completed {
                    return false;
                }
                let voxels = self.create_block_buffer();
                self.lods[0].map.set_block_buffer(block_pos, voxels, true);
            }
        }

        self.lods[0].map.set_voxel(value, pos, channel_index);
        true
    }

    fn create_block_buffer(&self) -> VoxelBuffer {
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(self.block_size() as i32));
        self.format.configure_buffer(&mut voxels);
        voxels
    }

    pub fn try_get_block_voxels(&self, block_pos: Vector3i) -> Option<&VoxelBuffer> {
        self.get_block(block_pos, 0).and_then(|block| {
            if block.has_voxels() {
                Some(block.voxels())
            } else {
                None
            }
        })
    }

    pub fn try_set_voxel_f(&mut self, value: f32, pos: Vector3i, channel_index: usize) -> bool {
        let raw = real_to_raw_voxel(value, self.format.depths[channel_index]);
        self.try_set_voxel(raw, pos, channel_index)
    }

    pub fn try_set_block(&mut self, block_pos: Vector3i, block: VoxelDataBlock) -> bool {
        let lod_index = usize::from(block.lod_index());
        assert!(lod_index < self.lods.len(), "block LOD is not loaded");
        if block.has_voxels() {
            assert_eq!(
                block.voxels().size(),
                Vector3i::splat(self.block_size() as i32),
                "block voxels must match VoxelData block size"
            );
        }
        if self.lods[lod_index].map.has_block(block_pos) {
            return false;
        }
        match block.into_voxels() {
            Some(voxels) => {
                self.lods[lod_index]
                    .map
                    .set_block_buffer(block_pos, voxels, false);
            }
            None => {
                self.lods[lod_index].map.set_empty_block(block_pos, false);
            }
        }
        true
    }

    pub fn has_block(&self, block_pos: Vector3i, lod_index: usize) -> bool {
        self.lods
            .get(lod_index)
            .is_some_and(|lod| lod.map.has_block(block_pos))
    }

    pub fn block_count(&self) -> usize {
        self.lods.iter().map(|lod| lod.map.block_count()).sum()
    }

    pub fn mark_area_modified(
        &mut self,
        voxel_box: Box3i,
        require_lod_updates: bool,
    ) -> Vec<Vector3i> {
        let blocks_box = voxel_box.downscaled(self.block_size() as i32);
        let mut newly_needing_lod = Vec::new();
        for block_pos in blocks_box.iter_cells_zxy() {
            let Some(block) = self.lods[0].map.get_block_mut(block_pos) else {
                continue;
            };
            if !block.has_voxels() {
                continue;
            }
            block.set_modified(true);
            block.set_edited(true);
            if require_lod_updates && !block.needs_lodding() {
                block.set_needs_lodding(true);
                newly_needing_lod.push(block_pos);
            }
        }
        newly_needing_lod
    }

    pub fn get_block(&self, block_pos: Vector3i, lod_index: usize) -> Option<&VoxelDataBlock> {
        self.lods
            .get(lod_index)
            .and_then(|lod| lod.map.get_block(block_pos))
    }
}

#[cfg(test)]
mod tests {
    use super::VoxelData;
    use crate::math::{Box3i, Vector3i};
    use crate::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelDataBlock, VoxelFormat};

    #[test]
    fn lod_count_resizes_maps_and_reset_preserves_settings() {
        let mut data = VoxelData::new();
        assert_eq!(data.lod_count(), 1);

        data.set_lod_count(3);
        assert_eq!(data.lod_count(), 3);
        assert_eq!(data.block_count(), 0);

        let bounds = Box3i::new(Vector3i::new(-16, -16, -16), Vector3i::new(32, 32, 32));
        data.set_bounds(bounds);
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();
        assert!(data.try_set_voxel(11, Vector3i::zero(), channel));
        assert_eq!(data.block_count(), 1);

        data.reset_maps();

        assert_eq!(data.lod_count(), 3);
        assert_eq!(data.bounds(), bounds);
        assert!(data.is_full_load_completed());
        assert_eq!(data.block_count(), 0);
    }

    #[test]
    fn set_format_resets_maps_and_configures_new_blocks() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        assert!(data.try_set_voxel(1, Vector3i::zero(), ChannelId::Type.index()));
        assert_eq!(data.block_count(), 1);

        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        data.set_format(format);

        assert_eq!(data.block_count(), 0);
        assert_eq!(data.format(), format);
        assert!(data.try_set_voxel_f(-3.25, Vector3i::zero(), ChannelId::Sdf.index()));
        let block = data.get_block(Vector3i::zero(), 0).unwrap();
        assert_eq!(
            block.voxels().channel_depth(ChannelId::Sdf.index()),
            ChannelDepth::Bit32
        );
    }

    #[test]
    fn try_set_voxel_requires_bounds_and_known_loaded_data() {
        let mut data = VoxelData::new();
        let channel = ChannelId::Type.index();
        let inside = Vector3i::new(1, 1, 1);
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::new(4, 4, 4)));

        assert!(data.is_streaming_enabled());
        assert!(!data.try_set_voxel(5, inside, channel));
        assert_eq!(data.get_voxel(inside, channel, 99), 99);

        data.set_full_load_completed(true);
        assert!(!data.try_set_voxel(5, inside, channel));

        data.set_streaming_enabled(false);

        assert!(data.try_set_voxel(5, inside, channel));
        assert_eq!(data.get_voxel(inside, channel, 99), 5);
        assert!(!data.try_set_voxel(6, Vector3i::new(8, 1, 1), channel));
        assert_eq!(data.get_voxel(Vector3i::new(8, 1, 1), channel, 99), 99);
    }

    #[test]
    fn try_set_block_inserts_once_and_tracks_lod() {
        let mut data = VoxelData::new();
        data.set_lod_count(2);
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(data.block_size() as i32));
        voxels.set_voxel(7, 0, 0, 0, ChannelId::Type.index());
        let block = VoxelDataBlock::with_voxels(voxels, 1);
        let block_pos = Vector3i::new(3, 0, -2);

        assert!(data.try_set_block(block_pos, block));
        assert!(data.has_block(block_pos, 1));
        assert_eq!(data.block_count(), 1);

        let duplicate = VoxelDataBlock::empty(1);
        assert!(!data.try_set_block(block_pos, duplicate));
        assert_eq!(data.block_count(), 1);
    }

    #[test]
    fn streaming_try_set_voxel_requires_existing_block() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)));
        let channel = ChannelId::Type.index();
        let pos = Vector3i::new(1, 1, 1);

        assert!(!data.try_set_voxel(3, pos, channel));

        let voxels = VoxelBuffer::with_size(Vector3i::splat(data.block_size() as i32));
        assert!(data.try_set_block(Vector3i::zero(), VoxelDataBlock::with_voxels(voxels, 0)));

        assert!(data.try_set_voxel(3, pos, channel));
        assert_eq!(data.get_voxel(pos, channel, 99), 3);
        assert!(data.try_get_block_voxels(Vector3i::zero()).is_some());
    }

    #[test]
    fn mark_area_modified_sets_block_flags_once() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::new(64, 16, 16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), ChannelId::Type.index()));
        assert!(data.try_set_voxel(2, Vector3i::new(20, 1, 1), ChannelId::Type.index()));

        let changed = data.mark_area_modified(
            Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)),
            true,
        );

        assert_eq!(
            changed,
            vec![Vector3i::new(0, 0, 0), Vector3i::new(1, 0, 0)]
        );
        for block_pos in changed {
            let block = data.get_block(block_pos, 0).unwrap();
            assert!(block.is_modified());
            assert!(block.is_edited());
            assert!(block.needs_lodding());
        }

        let second = data.mark_area_modified(
            Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)),
            true,
        );
        assert!(second.is_empty());
    }
}
