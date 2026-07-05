//! Aggregate voxel storage over LOD maps.
//!
//! First-pass, engine-agnostic port of `storage/voxel_data.{h,cpp}`. This file
//! intentionally starts with the synchronous storage contract: LOD maps, format,
//! bounds, block insertion, direct voxel edits and modification flags. Generator
//! and stream task integration are layered on top in later Phase 4 steps.

use crate::constants::voxel_constants::MAX_LOD;
use crate::generators::base::{VoxelGenerator, VoxelQueryData};
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
pub struct BlockToSave {
    pub voxels: Option<VoxelBuffer>,
    pub position: Vector3i,
    pub lod_index: u8,
}

/// Position of a block affected by a LOD update pass.
/// Matches `VoxelData::BlockLocation` in C++.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockLocation {
    pub position: Vector3i,
    pub lod_index: u8,
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
        self.lods[lod_index].map.set_block(block_pos, block, false);
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

    /// Propagates LOD0 edits to higher LODs by 2:1 downscaling.
    ///
    /// Ports `VoxelData::update_lods`. The caller passes the LOD0 blocks that
    /// were marked as needing LOD updates (typically the result of
    /// [`mark_area_modified`]). The function walks up the LOD chain in pairs:
    /// for each source (lower-LOD) block it finds or generates the destination
    /// (higher-LOD) block, marks it modified, and downscales the source
    /// voxels into the matching sub-region of the destination.
    ///
    /// When `generator` is `Some`, missing or empty destination blocks in
    /// non-streaming mode are filled by the generator before downscaling
    /// (matching the C++ `L::generate_voxels` path). In streaming mode the
    /// destination is expected to already be resident; if not, the function
    /// logs the discrepancy and skips that pair (the C++ branch prints an
    /// error and continues).
    ///
    /// If `out_updated_blocks` is `Some`, every block touched at every LOD is
    /// appended (LOD0 first, then progressively higher LODs). This mirrors
    /// the C++ `StdVector<BlockLocation> *out_updated_blocks` parameter.
    pub fn update_lods(
        &mut self,
        modified_lod0_blocks: &[Vector3i],
        mut generator: Option<&mut dyn VoxelGenerator>,
        mut out_updated_blocks: Option<&mut Vec<BlockLocation>>,
    ) {
        let lod_count = self.lods.len();
        if lod_count < 2 && modified_lod0_blocks.is_empty() {
            // Single-LOD case still needs to clear the needs_lodding flag so
            // the caller doesn't see stale state; handled below.
        }

        // Per-LOD worklists. Index 0 is seeded from the caller's input; each
        // successive LOD is filled by the cascade. Using a small fixed-size
        // `Vec<Vec<_>>` mirrors the C++ `thread_local FixedArray<...,MAX_LOD>`.
        let mut blocks_to_process_per_lod: Vec<Vec<Vector3i>> =
            (0..lod_count).map(|i| if i == 0 { modified_lod0_blocks.to_vec() } else { Vec::new() }).collect();

        // LOD0 phase: clear needs_lodding and record updates.
        for &block_pos in &blocks_to_process_per_lod[0] {
            let Some(block) = self.lods[0].map.get_block_mut(block_pos) else {
                // C++ uses ERR_CONTINUE; we just skip the missing block.
                continue;
            };
            block.set_needs_lodding(false);
            if let Some(out) = out_updated_blocks.as_deref_mut() {
                out.push(BlockLocation { position: block_pos, lod_index: 0 });
            }
        }

        let half_bs = (self.block_size() as i32) >> 1;
        let last_lod_index = lod_count - 1;

        // Cascade upwards in pairs of consecutive LODs.
        for dst_lod_index in 1..lod_count {
            let src_lod_index = dst_lod_index - 1;
            // Snapshot the src worklist so we can borrow `self` mutably inside
            // the loop without holding the borrow across iterations.
            let src_worklist = std::mem::take(&mut blocks_to_process_per_lod[src_lod_index]);

            for src_bpos in src_worklist {
                let dst_bpos = src_bpos >> 1;

                // Resolve the source block. C++ asserts non-null; the input
                // contract guarantees the block exists (it came from a
                // `needs_lodding` flag set by mark_area_modified).
                let src_has_voxels = self.lods[src_lod_index]
                    .map
                    .get_block(src_bpos)
                    .is_some_and(|block| block.has_voxels());
                if !src_has_voxels {
                    // Source block missing or empty — nothing to downscale.
                    continue;
                }

                // Resolve (or generate) the destination block.
                let dst_exists = self.lods[dst_lod_index].map.has_block(dst_bpos);
                if !dst_exists {
                    if !self.streaming_enabled {
                        // Generate an empty destination block and fill it via
                        // the generator before downscaling. Matches C++.
                        let mut voxels = self.create_block_buffer();
                        if let Some(generator) = generator.as_deref_mut() {
                            let lod_block_size = (self.block_size() as i32) << dst_lod_index;
                            generator.generate_block(VoxelQueryData {
                                buffer: &mut voxels,
                                origin_in_voxels: dst_bpos * lod_block_size,
                                lod: dst_lod_index as u32,
                            });
                        }
                        self.lods[dst_lod_index]
                            .map
                            .set_block_buffer(dst_bpos, voxels, true);
                    } else {
                        // Streaming mode expects parents to be resident. The
                        // C++ branch prints an error and `continue`s.
                        // TODO: route via the project logger once integrated.
                        continue;
                    }
                }

                // The destination may still have no voxel buffer (loaded but
                // uncached). Generate on the fly like C++.
                let dst_has_voxels = self.lods[dst_lod_index]
                    .map
                    .get_block(dst_bpos)
                    .is_some_and(|block| block.has_voxels());
                if !dst_has_voxels {
                    let mut voxels = self.create_block_buffer();
                    if let Some(generator) = generator.as_deref_mut() {
                        let lod_block_size = (self.block_size() as i32) << dst_lod_index;
                        generator.generate_block(VoxelQueryData {
                            buffer: &mut voxels,
                            origin_in_voxels: dst_bpos * lod_block_size,
                            lod: dst_lod_index as u32,
                        });
                    }
                    if let Some(block) = self.lods[dst_lod_index].map.get_block_mut(dst_bpos) {
                        block.set_voxels(voxels);
                    }
                }

                // Mark modified and enqueue for the next LOD pass if needed.
                let mut enqueue_next = false;
                if let Some(block) = self.lods[dst_lod_index].map.get_block_mut(dst_bpos) {
                    block.set_modified(true);
                    if dst_lod_index != last_lod_index && !block.needs_lodding() {
                        block.set_needs_lodding(true);
                        enqueue_next = true;
                    }
                }
                if enqueue_next {
                    blocks_to_process_per_lod[dst_lod_index].push(dst_bpos);
                }

                if let Some(out) = out_updated_blocks.as_deref_mut() {
                    out.push(BlockLocation {
                        position: dst_bpos,
                        lod_index: dst_lod_index as u8,
                    });
                }

                // Downscale source into the matching sub-region of the dst.
                // `rel = src_bpos - (dst_bpos << 1)` selects one of the 2×2×2
                // octants of the destination block; scaled by `half_bs` it
                // gives the destination-local offset of that octant.
                let rel = src_bpos - (dst_bpos << 1);
                let dst_offset = rel * half_bs;

                // Borrow src and dst blocks independently. `src_lod_index` is
                // always less than `dst_lod_index`, so we split the LOD slice
                // to convince the borrow checker the two borrows are disjoint.
                let (src_lods, dst_lods) = self.lods.split_at_mut(dst_lod_index);
                let Some(src_block) = src_lods[src_lod_index].map.get_block(src_bpos) else {
                    continue;
                };
                let Some(dst_block) = dst_lods[0].map.get_block_mut(dst_bpos) else {
                    continue;
                };

                // Copy the source voxels into a temporary so we don't hold a
                // borrow of `src_block` while mutating `dst_block` (the two
                // live in different LOD maps but share the same `&mut self`).
                // `downscale_to` takes `&self` and `&mut dst`, and our two
                // references come from disjoint LOD slices, so this is sound.
                let src_size = src_block.voxels().size();
                let dst_voxels = dst_block.voxels_mut();
                src_block.voxels().downscale_to(dst_voxels, Vector3i::zero(), src_size, dst_offset);
            }
        }
    }

    pub fn pre_generate_box(
        &mut self,
        voxel_box: Box3i,
        mut generator: Option<&mut dyn VoxelGenerator>,
    ) -> usize {
        let mut generated_count = 0;
        let data_block_size = self.block_size() as i32;
        for lod_index in 0..self.lods.len() {
            let lod_block_size = data_block_size << lod_index;
            let block_box = voxel_box.downscaled(lod_block_size);
            for block_pos in block_box.iter_cells_zxy() {
                let should_generate = match self.lods[lod_index].map.get_block(block_pos) {
                    Some(block) => !block.has_voxels(),
                    None => !self.streaming_enabled,
                };
                if !should_generate {
                    continue;
                }

                let mut voxels = self.create_block_buffer();
                if let Some(generator) = generator.as_deref_mut() {
                    generator.generate_block(VoxelQueryData {
                        buffer: &mut voxels,
                        origin_in_voxels: block_pos * lod_block_size,
                        lod: lod_index as u32,
                    });
                }

                if self.lods[lod_index]
                    .map
                    .get_block(block_pos)
                    .is_some_and(|block| block.has_voxels())
                {
                    continue;
                }

                self.lods[lod_index]
                    .map
                    .set_block_buffer(block_pos, voxels, true);
                generated_count += 1;
            }
        }
        generated_count
    }

    pub fn consume_block_modifications(&mut self, block_pos: Vector3i) -> Option<BlockToSave> {
        self.consume_block_modifications_at(block_pos, 0)
    }

    pub fn consume_all_modifications(&mut self) -> Vec<BlockToSave> {
        let mut saves = Vec::new();
        for lod_index in 0..self.lods.len() {
            let block_positions: Vec<_> = self.lods[lod_index].map.block_positions().collect();
            for block_pos in block_positions {
                if let Some(save) = self.consume_block_modifications_at(block_pos, lod_index) {
                    saves.push(save);
                }
            }
        }
        saves
    }

    fn consume_block_modifications_at(
        &mut self,
        block_pos: Vector3i,
        lod_index: usize,
    ) -> Option<BlockToSave> {
        let lod = self.lods.get_mut(lod_index)?;
        let block = lod.map.get_block_mut(block_pos)?;
        if !block.is_modified() {
            return None;
        }
        let voxels = if block.has_voxels() {
            Some(block.voxels().copy_to_owned())
        } else {
            None
        };
        block.set_modified(false);
        Some(BlockToSave {
            voxels,
            position: block_pos,
            lod_index: lod_index as u8,
        })
    }

    pub fn unload_blocks(
        &mut self,
        blocks_box: Box3i,
        lod_index: usize,
        collect_modified: bool,
    ) -> Vec<BlockToSave> {
        let Some(lod) = self.lods.get_mut(lod_index) else {
            return Vec::new();
        };
        let mut saves = Vec::new();
        for block_pos in blocks_box.iter_cells_zxy() {
            let Some(block) = lod.map.remove_block(block_pos) else {
                continue;
            };
            if collect_modified && block.is_modified() {
                saves.push(BlockToSave {
                    voxels: block.into_voxels(),
                    position: block_pos,
                    lod_index: lod_index as u8,
                });
            }
        }
        saves
    }

    pub fn get_block(&self, block_pos: Vector3i, lod_index: usize) -> Option<&VoxelDataBlock> {
        self.lods
            .get(lod_index)
            .and_then(|lod| lod.map.get_block(block_pos))
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockLocation, VoxelData};
    use crate::generators::base::{GenResult, VoxelGenerator, VoxelQueryData};
    use crate::math::{Box3i, Vector3i};
    use crate::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelDataBlock, VoxelFormat};

    #[derive(Default)]
    struct RecordingGenerator {
        calls: Vec<(Vector3i, u32)>,
    }

    impl VoxelGenerator for RecordingGenerator {
        fn generate_block(&mut self, input: VoxelQueryData<'_>) -> GenResult {
            self.calls.push((input.origin_in_voxels, input.lod));
            let value = 10 + input.lod as u64 + input.origin_in_voxels.x as u64;
            input.buffer.fill(value, ChannelId::Type.index());
            GenResult::default()
        }

        fn used_channels_mask(&self) -> u32 {
            1 << ChannelId::Type.index()
        }
    }

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

    #[test]
    fn pre_generate_box_non_streaming_generates_missing_lod_blocks() {
        let mut data = VoxelData::new();
        data.set_lod_count(2);
        data.set_streaming_enabled(false);
        let mut generator = RecordingGenerator::default();

        let generated = data.pre_generate_box(
            Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)),
            Some(&mut generator),
        );

        assert_eq!(generated, 3);
        assert_eq!(
            generator.calls,
            vec![
                (Vector3i::new(0, 0, 0), 0),
                (Vector3i::new(16, 0, 0), 0),
                (Vector3i::new(0, 0, 0), 1),
            ]
        );
        assert_eq!(
            data.get_block(Vector3i::new(1, 0, 0), 0)
                .unwrap()
                .voxels()
                .get_voxel(0, 0, 0, ChannelId::Type.index()),
            26
        );
        assert_eq!(
            data.get_block(Vector3i::zero(), 1)
                .unwrap()
                .voxels()
                .get_voxel(0, 0, 0, ChannelId::Type.index()),
            11
        );
    }

    #[test]
    fn pre_generate_box_streaming_only_fills_existing_empty_blocks() {
        let mut data = VoxelData::new();
        let block_pos = Vector3i::zero();
        assert!(data.try_set_block(block_pos, VoxelDataBlock::empty(0)));
        let mut generator = RecordingGenerator::default();

        let generated = data.pre_generate_box(
            Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)),
            Some(&mut generator),
        );

        assert_eq!(generated, 1);
        assert!(data.try_get_block_voxels(block_pos).is_some());
        assert!(!data.has_block(Vector3i::new(1, 0, 0), 0));
    }

    #[test]
    fn consume_block_modifications_copies_voxels_and_clears_modified_flag() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::new(16, 16, 16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();
        assert!(data.try_set_voxel(7, Vector3i::new(1, 1, 1), channel));
        data.mark_area_modified(
            Box3i::new(Vector3i::zero(), Vector3i::new(16, 16, 16)),
            false,
        );

        let mut save = data
            .consume_block_modifications(Vector3i::zero())
            .expect("modified block should be consumed");

        assert_eq!(save.position, Vector3i::zero());
        assert_eq!(save.lod_index, 0);
        assert_eq!(save.voxels.as_ref().unwrap().get_voxel(1, 1, 1, channel), 7);
        save.voxels.as_mut().unwrap().set_voxel(9, 1, 1, 1, channel);
        assert_eq!(data.get_voxel(Vector3i::new(1, 1, 1), channel, 99), 7);
        assert!(!data.get_block(Vector3i::zero(), 0).unwrap().is_modified());
        assert!(data.consume_block_modifications(Vector3i::zero()).is_none());
    }

    #[test]
    fn consume_all_modifications_collects_all_lods() {
        let mut data = VoxelData::new();
        data.set_lod_count(2);
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::new(16, 16, 16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        assert!(data.try_set_voxel(3, Vector3i::new(1, 1, 1), ChannelId::Type.index()));
        data.mark_area_modified(
            Box3i::new(Vector3i::zero(), Vector3i::new(16, 16, 16)),
            false,
        );

        let mut lod1_voxels = VoxelBuffer::with_size(Vector3i::splat(data.block_size() as i32));
        lod1_voxels.set_voxel(4, 0, 0, 0, ChannelId::Type.index());
        let mut lod1_block = VoxelDataBlock::with_voxels(lod1_voxels, 1);
        lod1_block.set_modified(true);
        assert!(data.try_set_block(Vector3i::new(2, 0, 0), lod1_block));

        let saves = data.consume_all_modifications();

        assert_eq!(saves.len(), 2);
        assert!(saves
            .iter()
            .any(|save| save.position == Vector3i::zero() && save.lod_index == 0));
        assert!(saves
            .iter()
            .any(|save| save.position == Vector3i::new(2, 0, 0) && save.lod_index == 1));
        assert!(!data.get_block(Vector3i::zero(), 0).unwrap().is_modified());
        assert!(!data
            .get_block(Vector3i::new(2, 0, 0), 1)
            .unwrap()
            .is_modified());
    }

    #[test]
    fn unload_blocks_removes_blocks_and_returns_modified_voxels_to_save() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        assert!(data.try_set_voxel(5, Vector3i::new(1, 1, 1), ChannelId::Type.index()));
        assert!(data.try_set_voxel(6, Vector3i::new(20, 1, 1), ChannelId::Type.index()));
        data.mark_area_modified(
            Box3i::new(Vector3i::zero(), Vector3i::new(16, 16, 16)),
            false,
        );

        let saves = data.unload_blocks(
            Box3i::new(Vector3i::zero(), Vector3i::new(2, 1, 1)),
            0,
            true,
        );

        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].position, Vector3i::zero());
        assert!(saves[0].voxels.is_some());
        assert!(!data.has_block(Vector3i::zero(), 0));
        assert!(!data.has_block(Vector3i::new(1, 0, 0), 0));
    }

    #[test]
    fn update_lods_clears_needs_lodding_and_reports_touched_blocks() {
        let mut data = VoxelData::new();
        data.set_lod_count(2);
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);

        // Two LOD0 blocks need LOD updates.
        let channel = ChannelId::Type.index();
        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), channel));
        assert!(data.try_set_voxel(2, Vector3i::new(20, 1, 1), channel));
        let modified = data.mark_area_modified(
            Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)),
            true,
        );
        assert_eq!(modified.len(), 2);

        let mut updated = Vec::new();
        data.update_lods(&modified, None, Some(&mut updated));

        // LOD0 blocks: needs_lodding cleared and reported.
        for &lod0_pos in &modified {
            assert!(!data.get_block(lod0_pos, 0).unwrap().needs_lodding());
        }
        // Both LOD0 positions map to the same LOD1 block (0,0,0).
        assert!(updated.contains(&BlockLocation {
            position: Vector3i::zero(),
            lod_index: 0,
        }));
        assert!(updated.contains(&BlockLocation {
            position: Vector3i::new(1, 0, 0),
            lod_index: 0,
        }));
        assert!(updated.contains(&BlockLocation {
            position: Vector3i::zero(),
            lod_index: 1,
        }));
        // The destination LOD1 block is now modified.
        assert!(data.get_block(Vector3i::zero(), 1).unwrap().is_modified());
    }

    #[test]
    fn update_lods_downscales_lod0_edits_into_lod1_octants() {
        let mut data = VoxelData::new();
        data.set_lod_count(2);
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();

        // Edit a single LOD0 voxel inside block (1,0,0). This block maps to
        // the +X octant of LOD1 block (0,0,0). Local coords (4,4,6) are chosen
        // so the 2:1 nearest-neighbor sample lands at LOD1 (10,2,3).
        let edited_pos = Vector3i::new(20, 4, 6);
        assert!(data.try_set_voxel(7, edited_pos, channel));
        let modified = data.mark_area_modified(
            Box3i::new(edited_pos, edited_pos + Vector3i::splat(1)),
            true,
        );
        assert_eq!(modified, vec![Vector3i::new(1, 0, 0)]);

        // Pre-create the destination LOD1 block so downscaling lands in it
        // (matches the streaming-pyramid invariant that parents are resident).
        let lod1_voxels = VoxelBuffer::with_size(Vector3i::splat(data.block_size() as i32));
        assert!(data.try_set_block(
            Vector3i::zero(),
            VoxelDataBlock::with_voxels(lod1_voxels, 1),
        ));

        data.update_lods(&modified, None, None);

        // The edited LOD0 voxel (20,4,6) maps to LOD1 (10,2,3) via 2:1 nearest.
        // In LOD1 block-local coords (block_size 16) that is (10,2,3).
        let lod1_block = data.get_block(Vector3i::zero(), 1).unwrap();
        assert_eq!(lod1_block.voxels().get_voxel(10, 2, 3, channel), 7);
        // A voxel outside the downscaled octant stays at the default.
        assert_eq!(lod1_block.voxels().get_voxel(0, 0, 0, channel), 0);
    }

    #[test]
    fn update_lods_generates_missing_destination_in_non_streaming_mode() {
        let mut data = VoxelData::new();
        data.set_lod_count(2);
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();

        assert!(data.try_set_voxel(11, Vector3i::new(1, 1, 1), channel));
        let modified = data.mark_area_modified(
            Box3i::new(Vector3i::zero(), Vector3i::new(16, 16, 16)),
            true,
        );

        // The destination LOD1 block doesn't exist; the generator must fill it
        // before the downscale runs. The recorder lets us observe the call.
        let mut generator = RecordingGenerator::default();
        data.update_lods(&modified, Some(&mut generator), None);

        // LOD1 block (0,0,0) was generated on demand and is now present.
        assert!(data.has_block(Vector3i::zero(), 1));
        assert!(generator.calls.iter().any(|(origin, lod)| {
            *lod == 1 && origin.x == 0 && origin.y == 0 && origin.z == 0
        }));
    }
}
