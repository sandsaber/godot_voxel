//! Aggregate voxel storage over LOD maps.
//!
//! Engine-agnostic port of `storage/voxel_data.{h,cpp}`. Owns the per-LOD
//! sparse block maps plus an optional generator and stream, and exposes the
//! synchronous storage contract: LOD maps, format, bounds, block insertion,
//! direct voxel edits, modification flags, LOD cascade, copy/paste, the
//! reference-counted view/unview API and area-loaded queries. Threaded
//! streaming task integration is layered on top in later Phase 4 steps.
//! Shared task code reaches `VoxelData` through [`SharedVoxelData`], which
//! owns the scoped `SpatialLock3D` region guards used by C++ terrain workers.

use crate::constants::voxel_constants::MAX_LOD;
use crate::generators::base::{VoxelGenerator, VoxelQueryData};
use crate::math::{Box3i, BoxBounds3i, Vector3i};
use crate::storage::{
    voxel_buffer::{raw_voxel_to_real, real_to_raw_voxel, SDF_FAR_OUTSIDE},
    VoxelBuffer, VoxelDataBlock, VoxelDataMap, VoxelFormat,
};
use crate::streams::VoxelStream;
use crate::thread::SpatialLock3D;
use std::fmt;
use std::sync::{
    Arc, RwLock as StdRwLock, RwLockReadGuard as StdRwLockReadGuard,
    RwLockWriteGuard as StdRwLockWriteGuard, TryLockError,
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

/// Shared generator storage. Implementations are `Send + Sync` and own any
/// internal synchronization they need, matching the C++ contract that
/// generators can be called from multiple worker threads.
pub type SharedVoxelGenerator = Arc<dyn VoxelGenerator>;

/// Shared stream storage. `VoxelStream` is already `Send + Sync`; the `Arc`
/// lets multiple task instances reach the same stream.
pub type SharedVoxelStream = Arc<dyn VoxelStream>;

/// Test-only checkpoints for the transactional `SharedVoxelData` edit path.
///
/// When `try_edit_voxel` is implemented, it must notify these phases in order:
/// first after acquiring the spatial write region and before taking a LOD map
/// lock, then after `map.set_voxel` but before modification flags, and finally
/// after modification flags while still inside the same map write closure.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedVoxelDataEditPhase {
    SpatialWriteAcquiredBeforeMapLock,
    VoxelWrittenBeforeDirtyFlags,
    DirtyFlagsSetBeforeMapWriteUnlock,
}

#[cfg(test)]
pub type SharedVoxelDataEditPhaseHook =
    Arc<dyn Fn(SharedVoxelDataEditPhase) + Send + Sync + 'static>;

#[derive(Debug)]
struct SharedVoxelDataLod {
    map: StdRwLock<VoxelDataMap>,
}

struct SharedVoxelDataSettings {
    format: VoxelFormat,
    bounds_in_voxels: Box3i,
    full_load_completed: bool,
    streaming_enabled: bool,
    generator: Option<SharedVoxelGenerator>,
    stream: Option<SharedVoxelStream>,
}

#[derive(Clone)]
pub struct SharedVoxelDataSettingsSnapshot {
    pub format: VoxelFormat,
    pub bounds_in_voxels: Box3i,
    pub full_load_completed: bool,
    pub streaming_enabled: bool,
    pub generator: Option<SharedVoxelGenerator>,
    pub stream: Option<SharedVoxelStream>,
}

/// Shared voxel-data handle for worker tasks.
///
/// This is the migration boundary between the earlier `Arc<Mutex<VoxelData>>`
/// port and the C++ shape where terrain code passes a shared `VoxelData`
/// pointer and each method scopes its own map/region locks. Settings now live
/// behind their own lock, each LOD map has an independent lock, and the
/// per-LOD [`SpatialLock3D`] guards are taken by mesh/read and edit/write
/// regions before touching voxel data.
pub struct SharedVoxelData {
    lods: Vec<SharedVoxelDataLod>,
    settings: StdRwLock<SharedVoxelDataSettings>,
    spatial_locks: Vec<SpatialLock3D>,
    #[cfg(test)]
    edit_phase_hook: StdRwLock<Option<SharedVoxelDataEditPhaseHook>>,
}

impl SharedVoxelData {
    pub fn new(data: VoxelData) -> Self {
        let VoxelData {
            lods,
            format,
            bounds_in_voxels,
            full_load_completed,
            streaming_enabled,
            generator,
            stream,
        } = data;
        Self {
            lods: lods
                .into_iter()
                .map(|lod| SharedVoxelDataLod {
                    map: StdRwLock::new(lod.map),
                })
                .collect(),
            settings: StdRwLock::new(SharedVoxelDataSettings {
                format,
                bounds_in_voxels,
                full_load_completed,
                streaming_enabled,
                generator,
                stream,
            }),
            spatial_locks: (0..MAX_LOD).map(|_| SpatialLock3D::new()).collect(),
            #[cfg(test)]
            edit_phase_hook: StdRwLock::new(None),
        }
    }

    pub const fn block_size(&self) -> u32 {
        VoxelDataMap::BLOCK_SIZE
    }

    pub const fn block_size_po2(&self) -> u8 {
        VoxelDataMap::BLOCK_SIZE_PO2
    }

    pub fn lod_count(&self) -> usize {
        self.lods.len()
    }

    pub fn settings_snapshot(&self) -> SharedVoxelDataSettingsSnapshot {
        let settings = self.settings.read().unwrap_or_else(|e| e.into_inner());
        SharedVoxelDataSettingsSnapshot {
            format: settings.format,
            bounds_in_voxels: settings.bounds_in_voxels,
            full_load_completed: settings.full_load_completed,
            streaming_enabled: settings.streaming_enabled,
            generator: settings.generator.clone(),
            stream: settings.stream.clone(),
        }
    }

    #[cfg(test)]
    fn with_settings<R>(&self, f: impl FnOnce(&SharedVoxelDataSettings) -> R) -> R {
        let settings = self.settings.read().unwrap_or_else(|e| e.into_inner());
        f(&settings)
    }

    /// Registers a test-only edit lifecycle observer.
    ///
    /// This has no production build surface. `try_edit_voxel` deliberately
    /// does not exist yet; Task 2 must call [`Self::notify_test_edit_phase`]
    /// at the ordered checkpoints documented on [`SharedVoxelDataEditPhase`].
    #[cfg(test)]
    pub fn set_test_edit_phase_hook(&self, hook: SharedVoxelDataEditPhaseHook) {
        *self
            .edit_phase_hook
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    #[cfg(test)]
    fn notify_test_edit_phase(&self, phase: SharedVoxelDataEditPhase) {
        let hook = self
            .edit_phase_hook
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook(phase);
        }
    }

    pub fn format(&self) -> VoxelFormat {
        self.settings_snapshot().format
    }

    pub fn bounds(&self) -> Box3i {
        self.settings_snapshot().bounds_in_voxels
    }

    pub fn generator(&self) -> Option<SharedVoxelGenerator> {
        self.settings_snapshot().generator
    }

    pub fn stream(&self) -> Option<SharedVoxelStream> {
        self.settings_snapshot().stream
    }

    pub fn set_generator(&self, generator: Option<SharedVoxelGenerator>) {
        self.settings
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .generator = generator;
    }

    pub fn with_lod_map<R>(&self, lod_index: usize, f: impl FnOnce(&VoxelDataMap) -> R) -> R {
        let lod = self
            .lods
            .get(lod_index)
            .expect("LOD index is outside the loaded range");
        let map = lod.map.read().unwrap_or_else(|e| e.into_inner());
        f(&map)
    }

    pub fn with_lod_map_mut<R>(
        &self,
        lod_index: usize,
        f: impl FnOnce(&mut VoxelDataMap) -> R,
    ) -> R {
        let lod = self
            .lods
            .get(lod_index)
            .expect("LOD index is outside the loaded range");
        let mut map = lod.map.write().unwrap_or_else(|e| e.into_inner());
        f(&mut map)
    }

    pub fn try_lock(&self) -> Option<StdRwLockWriteGuard<'_, VoxelDataMap>> {
        self.try_lod_map_write(0)
    }

    pub fn try_lod_map_write(
        &self,
        lod_index: usize,
    ) -> Option<StdRwLockWriteGuard<'_, VoxelDataMap>> {
        let lod = self.lods.get(lod_index)?;
        match lod.map.try_write() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    pub fn try_lod_map_read(
        &self,
        lod_index: usize,
    ) -> Option<StdRwLockReadGuard<'_, VoxelDataMap>> {
        let lod = self.lods.get(lod_index)?;
        match lod.map.try_read() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    pub fn has_all_blocks_in_area(&self, blocks_box: Box3i, lod_index: usize) -> bool {
        if lod_index >= self.lods.len() {
            return false;
        }
        self.with_lod_map(lod_index, |map| {
            blocks_box.all_cells_match(|pos| map.has_block(pos))
        })
    }

    pub fn try_set_block(&self, block_pos: Vector3i, block: VoxelDataBlock) -> bool {
        let lod_index = usize::from(block.lod_index());
        assert!(lod_index < self.lods.len(), "block LOD is not loaded");
        if block.has_voxels() {
            assert_eq!(
                block.voxels().size(),
                Vector3i::splat(self.block_size() as i32),
                "block voxels must match VoxelData block size"
            );
        }
        self.with_lod_map_mut(lod_index, |map| {
            if map.has_block(block_pos) {
                return false;
            }
            map.set_block(block_pos, block, false);
            true
        })
    }

    pub fn view_area(
        &self,
        mut blocks_box: Box3i,
        lod_index: usize,
        missing_blocks: Option<&mut Vec<Vector3i>>,
        found_blocks_positions: Option<&mut Vec<Vector3i>>,
        found_blocks: Option<&mut Vec<VoxelDataBlock>>,
    ) {
        let bounds_in_blocks = self.bounds().downscaled(self.block_size() as i32);
        blocks_box = blocks_box.clipped(bounds_in_blocks);

        if lod_index >= self.lods.len() {
            return;
        }

        let mut missing_local = Vec::new();
        let mut found_positions_local = Vec::new();
        let mut found_blocks_local: Vec<VoxelDataBlock> = Vec::new();

        self.with_lod_map_mut(lod_index, |map| {
            for bpos in blocks_box.iter_cells_zxy() {
                match map.get_block_mut(bpos) {
                    Some(block) => {
                        block.viewers.add();
                        if found_blocks.is_some() {
                            found_blocks_local.push(clone_block(block));
                        }
                        if found_blocks_positions.is_some() {
                            found_positions_local.push(bpos);
                        }
                    }
                    None => {
                        if missing_blocks.is_some() {
                            missing_local.push(bpos);
                        }
                    }
                }
            }
        });

        if let Some(out) = missing_blocks {
            out.extend(missing_local);
        }
        if let Some(out) = found_blocks_positions {
            out.extend(found_positions_local);
        }
        if let Some(out) = found_blocks {
            out.extend(found_blocks_local);
        }
    }

    pub fn unview_area(
        &self,
        mut blocks_box: Box3i,
        lod_index: usize,
        removed_blocks: Option<&mut Vec<Vector3i>>,
        missing_blocks: Option<&mut Vec<Vector3i>>,
        mut to_save: Option<&mut Vec<BlockToSave>>,
    ) {
        let bounds_in_blocks = self.bounds().downscaled(self.block_size() as i32);
        blocks_box = blocks_box.clipped(bounds_in_blocks);

        if lod_index >= self.lods.len() {
            if let Some(out) = missing_blocks {
                out.extend(blocks_box.iter_cells_zxy());
            }
            return;
        }

        let mut removed_local = Vec::new();
        let mut missing_local = Vec::new();

        self.with_lod_map_mut(lod_index, |map| {
            for bpos in blocks_box.iter_cells_zxy() {
                let should_remove = match map.get_block_mut(bpos) {
                    Some(block) => {
                        block.viewers.remove();
                        block.viewers.get() == 0
                    }
                    None => {
                        missing_local.push(bpos);
                        continue;
                    }
                };

                if should_remove {
                    if let Some(block) = map.remove_block(bpos) {
                        if let Some(out) = to_save.as_deref_mut() {
                            if block.is_modified() {
                                out.push(BlockToSave {
                                    voxels: block.into_voxels(),
                                    position: bpos,
                                    lod_index: lod_index as u8,
                                });
                            }
                        }
                        removed_local.push(bpos);
                    }
                }
            }
        });

        if let Some(out) = removed_blocks {
            out.extend(removed_local);
        }
        if let Some(out) = missing_blocks {
            out.extend(missing_local);
        }
    }

    pub fn try_edit_voxel(&self, value: u64, pos: Vector3i, channel_index: usize) -> bool {
        let settings = self.settings_snapshot();
        if !settings.bounds_in_voxels.contains_point(pos) {
            return false;
        }

        let block_size = self.block_size() as i32;
        let block_pos = VoxelDataMap::voxel_to_block_b(pos, self.block_size_po2());
        let block_box = Box3i::new(block_pos * block_size, Vector3i::splat(block_size));
        let _write_region = self.write_region(0, block_box);
        #[cfg(test)]
        self.notify_test_edit_phase(SharedVoxelDataEditPhase::SpatialWriteAcquiredBeforeMapLock);

        let needs_materialization = self.with_lod_map(0, |map| {
            map.get_block(block_pos)
                .is_none_or(|block| !block.has_voxels())
        });
        if needs_materialization && (settings.streaming_enabled || !settings.full_load_completed) {
            return false;
        }

        let mut prepared = needs_materialization.then(|| {
            let mut voxels = create_block_buffer(block_size, settings.format);
            if let Some(generator) = settings.generator {
                generator.generate_block(VoxelQueryData {
                    buffer: &mut voxels,
                    origin_in_voxels: block_pos * block_size,
                    lod: 0,
                });
            }
            voxels
        });

        self.with_lod_map_mut(0, |map| {
            let has_resident_voxels = map
                .get_block(block_pos)
                .is_some_and(|block| block.has_voxels());
            if !has_resident_voxels {
                map.set_block_buffer(
                    block_pos,
                    prepared.take().expect("materialization was prepared"),
                    true,
                );
            }

            map.set_voxel(value, pos, channel_index);
            #[cfg(test)]
            self.notify_test_edit_phase(SharedVoxelDataEditPhase::VoxelWrittenBeforeDirtyFlags);
            let block = map
                .get_block_mut(block_pos)
                .expect("edited block exists after materialization");
            block.set_modified(true);
            block.set_edited(true);
            #[cfg(test)]
            self.notify_test_edit_phase(
                SharedVoxelDataEditPhase::DirtyFlagsSetBeforeMapWriteUnlock,
            );
        });
        true
    }

    pub fn try_set_voxel(&self, value: u64, pos: Vector3i, channel_index: usize) -> bool {
        let settings = self.settings_snapshot();
        if !settings.bounds_in_voxels.contains_point(pos) {
            return false;
        }
        let block_pos = VoxelDataMap::voxel_to_block_b(pos, self.block_size_po2());
        let block_size = self.block_size() as i32;
        self.with_lod_map_mut(0, |map| {
            let block_state = map.get_block(block_pos).map(|block| block.has_voxels());

            match block_state {
                Some(true) => {}
                Some(false) => {
                    let voxels = create_block_buffer(block_size, settings.format);
                    map.set_block_buffer(block_pos, voxels, true);
                }
                None => {
                    if settings.streaming_enabled || !settings.full_load_completed {
                        return false;
                    }
                    let voxels = create_block_buffer(block_size, settings.format);
                    map.set_block_buffer(block_pos, voxels, true);
                }
            }

            map.set_voxel(value, pos, channel_index);
            true
        })
    }

    pub fn mark_area_modified(&self, voxel_box: Box3i, require_lod_updates: bool) -> Vec<Vector3i> {
        let blocks_box = voxel_box.downscaled(self.block_size() as i32);
        let mut newly_needing_lod = Vec::new();
        self.with_lod_map_mut(0, |map| {
            for block_pos in blocks_box.iter_cells_zxy() {
                let Some(block) = map.get_block_mut(block_pos) else {
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
        });
        newly_needing_lod
    }

    pub fn block_snapshot(&self, block_pos: Vector3i, lod_index: usize) -> Option<VoxelDataBlock> {
        if lod_index >= self.lods.len() {
            return None;
        }
        self.with_lod_map(lod_index, |map| map.get_block(block_pos).map(clone_block))
    }

    pub fn block_count(&self) -> usize {
        self.lods
            .iter()
            .map(|lod| {
                lod.map
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .block_count()
            })
            .sum()
    }

    pub fn read_region(&self, lod_index: usize, voxel_box: Box3i) -> SharedVoxelDataReadRegion<'_> {
        let bounds = bounds_from_box(voxel_box);
        let lock = self.spatial_lock(lod_index);
        lock.lock_read(bounds);
        SharedVoxelDataReadRegion { lock, bounds }
    }

    pub fn try_read_region(
        &self,
        lod_index: usize,
        voxel_box: Box3i,
    ) -> Option<SharedVoxelDataReadRegion<'_>> {
        let bounds = bounds_from_box(voxel_box);
        let lock = self.spatial_lock(lod_index);
        if lock.try_lock_read(bounds) {
            Some(SharedVoxelDataReadRegion { lock, bounds })
        } else {
            None
        }
    }

    pub fn write_region(
        &self,
        lod_index: usize,
        voxel_box: Box3i,
    ) -> SharedVoxelDataWriteRegion<'_> {
        let bounds = bounds_from_box(voxel_box);
        let lock = self.spatial_lock(lod_index);
        lock.lock_write(bounds);
        SharedVoxelDataWriteRegion { lock, bounds }
    }

    pub fn try_write_region(
        &self,
        lod_index: usize,
        voxel_box: Box3i,
    ) -> Option<SharedVoxelDataWriteRegion<'_>> {
        let bounds = bounds_from_box(voxel_box);
        let lock = self.spatial_lock(lod_index);
        if lock.try_lock_write(bounds) {
            Some(SharedVoxelDataWriteRegion { lock, bounds })
        } else {
            None
        }
    }

    pub fn locked_region_count(&self, lod_index: usize) -> usize {
        self.spatial_lock(lod_index).locked_boxes_count()
    }

    fn spatial_lock(&self, lod_index: usize) -> &SpatialLock3D {
        self.spatial_locks
            .get(lod_index)
            .expect("LOD index is outside the supported range")
    }
}

impl fmt::Debug for SharedVoxelData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let settings = self.settings.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("SharedVoxelData")
            .field("lod_count", &self.lods.len())
            .field("format", &settings.format)
            .field("bounds_in_voxels", &settings.bounds_in_voxels)
            .field("streaming_enabled", &settings.streaming_enabled)
            .field("full_load_completed", &settings.full_load_completed)
            .field("has_generator", &settings.generator.is_some())
            .field("has_stream", &settings.stream.is_some())
            .field("spatial_lock_count", &self.spatial_locks.len())
            .finish()
    }
}

#[derive(Debug)]
pub struct SharedVoxelDataReadRegion<'a> {
    lock: &'a SpatialLock3D,
    bounds: BoxBounds3i,
}

impl Drop for SharedVoxelDataReadRegion<'_> {
    fn drop(&mut self) {
        self.lock.unlock_read(self.bounds);
    }
}

#[derive(Debug)]
pub struct SharedVoxelDataWriteRegion<'a> {
    lock: &'a SpatialLock3D,
    bounds: BoxBounds3i,
}

impl Drop for SharedVoxelDataWriteRegion<'_> {
    fn drop(&mut self) {
        self.lock.unlock_write(self.bounds);
    }
}

fn bounds_from_box(voxel_box: Box3i) -> BoxBounds3i {
    BoxBounds3i::from_box(voxel_box.position, voxel_box.size)
}

fn create_block_buffer(block_size: i32, format: VoxelFormat) -> VoxelBuffer {
    let mut voxels = VoxelBuffer::with_size(Vector3i::splat(block_size));
    format.configure_buffer(&mut voxels);
    voxels
}

/// Aggregate voxel storage.
///
/// Locking invariant for task code using [`SharedVoxelData`]: clone shared
/// generator/stream handles and copy cheap settings while holding the data
/// lock, then release the lock before calling generator, mesher or stream
/// methods. This mirrors the C++ contract where those shared resources are
/// thread-safe and not protected by the voxel-data map lock.
pub struct VoxelData {
    lods: Vec<VoxelDataLod>,
    format: VoxelFormat,
    bounds_in_voxels: Box3i,
    full_load_completed: bool,
    streaming_enabled: bool,
    generator: Option<SharedVoxelGenerator>,
    stream: Option<SharedVoxelStream>,
}

impl fmt::Debug for VoxelData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VoxelData")
            .field("lod_count", &self.lods.len())
            .field("format", &self.format)
            .field("bounds_in_voxels", &self.bounds_in_voxels)
            .field("streaming_enabled", &self.streaming_enabled)
            .field("full_load_completed", &self.full_load_completed)
            .field("has_generator", &self.generator.is_some())
            .field("has_stream", &self.stream.is_some())
            .finish()
    }
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
            generator: None,
            stream: None,
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

    /// Returns a clone of the shared generator handle, if any. Cheap (one Arc
    /// refcount bump). Matches `VoxelData::get_generator` in C++.
    pub fn generator(&self) -> Option<SharedVoxelGenerator> {
        self.generator.clone()
    }

    /// Installs a shared generator. Matches `VoxelData::set_generator`.
    /// Pass `None` to detach. The handle can be safely cloned into worker
    /// tasks later; generators own any internal synchronization they need.
    pub fn set_generator(&mut self, generator: Option<SharedVoxelGenerator>) {
        self.generator = generator;
    }

    /// Runs `f` against the installed generator. Returns
    /// `None` when no generator is set. Used by `pre_generate_box` /
    /// `update_lods` when the caller doesn't pass an explicit generator.
    pub fn with_generator<R>(&self, f: impl FnOnce(&dyn VoxelGenerator) -> R) -> Option<R> {
        self.generator.as_ref().map(|gen| f(gen.as_ref()))
    }

    /// Returns a clone of the shared stream handle, if any. Matches
    /// `VoxelData::get_stream` in C++.
    pub fn stream(&self) -> Option<SharedVoxelStream> {
        self.stream.clone()
    }

    /// Installs a shared stream. Matches `VoxelData::set_stream`.
    pub fn set_stream(&mut self, stream: Option<SharedVoxelStream>) {
        self.stream = stream;
    }

    pub const fn has_generator(&self) -> bool {
        self.generator.is_some()
    }

    pub const fn has_stream(&self) -> bool {
        self.stream.is_some()
    }

    /// Copies voxel data in a box from LOD0 into `dst_buffer`. Ports
    /// `VoxelData::copy`. `channels_mask` selects which channels are read;
    /// missing blocks produce the format default. When a generator is
    /// installed and `generate_missing` is true, missing blocks inside
    /// bounds are generated on the fly instead of falling back to defaults
    /// (mirrors the C++ generator callback path).
    pub fn copy(
        &self,
        min_pos: Vector3i,
        dst_buffer: &mut VoxelBuffer,
        channels_mask: u32,
        generate_missing: bool,
    ) {
        if channels_mask == 0 {
            return;
        }
        // Match C++: configure the destination buffer with our format first.
        self.format.configure_buffer(dst_buffer);

        let dst_size = dst_buffer.size();
        if dst_size.x <= 0 || dst_size.y <= 0 || dst_size.z <= 0 {
            return;
        }

        let block_size = self.block_size() as i32;
        let max_pos = min_pos + dst_size;
        let min_block_pos = VoxelDataMap::voxel_to_block_b(min_pos, self.block_size_po2());
        let max_block_pos =
            VoxelDataMap::voxel_to_block_b(max_pos - Vector3i::splat(1), self.block_size_po2())
                + Vector3i::splat(1);

        let channels: Vec<usize> = (0..8u32)
            .filter(|ci| (channels_mask & (1u32 << ci)) != 0)
            .map(|ci| ci as usize)
            .collect();

        for block_pos in Box3i::from_min_max(min_block_pos, max_block_pos).iter_cells_zxy() {
            let src_block_origin = block_pos * block_size;
            let dst_offset = src_block_origin - min_pos;

            // Loaded edited block: copy directly from its voxel buffer.
            if let Some(block) = self.lods[0].map.get_block(block_pos) {
                if block.has_voxels() {
                    for &channel_index in &channels {
                        dst_buffer.copy_channel_from_area(
                            block.voxels(),
                            Vector3i::zero(),
                            block.voxels().size(),
                            dst_offset,
                            channel_index,
                        );
                    }
                    continue;
                }
            }

            // Missing block: generate on the fly if a generator is available
            // and the area is inside bounds; otherwise leave the default.
            if generate_missing
                && self.generator.is_some()
                && self.bounds_in_voxels.contains_point(src_block_origin)
            {
                let mut scratch = self.create_block_buffer();
                self.with_generator(|gen| {
                    gen.generate_block(VoxelQueryData {
                        buffer: &mut scratch,
                        origin_in_voxels: src_block_origin,
                        lod: 0,
                    });
                });
                for &channel_index in &channels {
                    dst_buffer.copy_channel_from_area(
                        &scratch,
                        Vector3i::zero(),
                        scratch.size(),
                        dst_offset,
                        channel_index,
                    );
                }
            }
        }
    }

    /// Pastes `src_buffer` into LOD0 at `min_pos`. Ports `VoxelData::paste`.
    /// `channels_mask` selects which channels are written.
    /// `create_new_blocks` controls whether missing destination blocks are
    /// materialised (as formatted empty buffers) before writing.
    pub fn paste(
        &mut self,
        min_pos: Vector3i,
        src_buffer: &VoxelBuffer,
        channels_mask: u32,
        create_new_blocks: bool,
    ) {
        self.lods[0]
            .map
            .paste(min_pos, src_buffer, channels_mask, create_new_blocks);
    }

    /// Pastes `src_buffer` into LOD0 with a source mask. Ports
    /// `VoxelData::paste_masked`. Voxels of `src_buffer` whose
    /// `src_mask_channel` equals `src_mask_value` are skipped.
    pub fn paste_masked(
        &mut self,
        min_pos: Vector3i,
        src_buffer: &VoxelBuffer,
        channels_mask: u32,
        src_mask_channel: usize,
        src_mask_value: u64,
        create_new_blocks: bool,
    ) {
        self.lods[0].map.paste_masked(
            min_pos,
            src_buffer,
            channels_mask,
            src_mask_channel,
            src_mask_value,
            create_new_blocks,
        );
    }

    /// Pastes `src_buffer` into LOD0 with a source mask and a destination
    /// writable-values list. Ports `VoxelData::paste_masked_writable_list`.
    /// Voxels of `src_buffer` whose `src_mask_channel` equals `src_mask_value`
    /// are skipped; voxels of the destination whose `dst_mask_channel` value
    /// is not in `dst_writable_values` are also skipped.
    #[allow(clippy::too_many_arguments)]
    pub fn paste_masked_with_destination_mask(
        &mut self,
        min_pos: Vector3i,
        src_buffer: &VoxelBuffer,
        channels_mask: u32,
        src_mask_channel: usize,
        src_mask_value: u64,
        dst_mask_channel: usize,
        dst_writable_values: &[u64],
        create_new_blocks: bool,
    ) {
        self.lods[0].map.paste_masked_with_destination_mask(
            min_pos,
            src_buffer,
            channels_mask,
            src_mask_channel,
            src_mask_value,
            dst_mask_channel,
            dst_writable_values,
            create_new_blocks,
        );
    }

    /// Tests whether every block intersecting the given voxel box at LOD0 is
    /// loaded. Ports `VoxelData::is_area_loaded`. The C++ version also
    /// short-circuits to false when streaming is enabled and the area
    /// extends outside the bounds (we replicate that here).
    pub fn is_area_loaded(&self, voxel_box: Box3i) -> bool {
        if self.streaming_enabled && !self.bounds_in_voxels.contains_box(voxel_box) {
            return false;
        }
        self.lods[0].map.is_area_fully_loaded(voxel_box)
    }

    /// Tests if all blocks in the given block-coord box at `lod_index` are
    /// loaded, accounting for data boundaries. Ports
    /// `VoxelData::has_all_blocks_in_area`.
    pub fn has_all_blocks_in_area(&self, blocks_box: Box3i, lod_index: usize) -> bool {
        let Some(lod) = self.lods.get(lod_index) else {
            return false;
        };
        blocks_box.all_cells_match(|pos| lod.map.has_block(pos))
    }

    /// Appends block positions inside `blocks_box` at `lod_index` that are
    /// not loaded. Ports `VoxelData::get_missing_blocks` (the box overload).
    pub fn get_missing_blocks(
        &self,
        blocks_box: Box3i,
        lod_index: usize,
        out_missing: &mut Vec<Vector3i>,
    ) {
        let Some(lod) = self.lods.get(lod_index) else {
            out_missing.extend(blocks_box.iter_cells_zxy());
            return;
        };
        for pos in blocks_box.iter_cells_zxy() {
            if !lod.map.has_block(pos) {
                out_missing.push(pos);
            }
        }
    }

    /// Returns references to the voxel buffers of every block with voxel data
    /// in `blocks_box` at `lod_index`, indexed into a flat ZXY grid covering
    /// the box. Missing or empty entries are left as `None`. Ports
    /// `VoxelData::get_blocks_with_voxel_data`.
    pub fn get_blocks_with_voxel_data(
        &self,
        blocks_box: Box3i,
        lod_index: usize,
    ) -> Vec<Option<&VoxelBuffer>> {
        let mut out = Vec::new();
        let Some(lod) = self.lods.get(lod_index) else {
            return out;
        };
        let size = blocks_box.size;
        out.reserve_exact((size.x as usize) * (size.y as usize) * (size.z as usize));
        for pos in blocks_box.iter_cells_zxy() {
            let buffer = lod
                .map
                .get_block(pos)
                .filter(|block| block.has_voxels())
                .map(|block| block.voxels());
            out.push(buffer);
        }
        out
    }

    pub fn get_voxel(&self, pos: Vector3i, channel_index: usize, defval: u64) -> u64 {
        if !self.bounds_in_voxels.contains_point(pos) {
            return defval;
        }
        if !self.streaming_enabled && !self.full_load_completed {
            return defval;
        }

        if !self.streaming_enabled {
            // Non-streaming: every block is expected to be loaded. If a block
            // or its voxels are missing, fall back to the generator (single
            // voxel query) — mirrors the C++ branch at voxel_data.cpp:182-200.
            let block_pos = self.voxel_to_block(pos);
            if let Some(block) = self.lods[0].map.get_block(block_pos) {
                if block.has_voxels() {
                    let local_pos = self.lods[0].map.to_local(pos);
                    return block.voxels().get_voxel(
                        local_pos.x,
                        local_pos.y,
                        local_pos.z,
                        channel_index,
                    );
                }
            }
            return self
                .with_generator(|gen| gen.generate_single(pos, channel_index).as_raw())
                .unwrap_or(defval);
        }

        // Streaming mode: probe LODs from finest to coarsest, falling back to
        // a lower LOD when the finer one isn't resident. If none is resident
        // and a generator is available, query it directly (matches the C++
        // behaviour at voxel_data.cpp:209-254).
        let mut block_pos = self.voxel_to_block(pos);
        let mut voxel_pos = pos;
        for lod_index in 0..self.lods.len() {
            if let Some(block) = self.lods[lod_index].map.get_block(block_pos) {
                if block.has_voxels() {
                    let local_pos = self.lods[lod_index].map.to_local(voxel_pos);
                    return block.voxels().get_voxel(
                        local_pos.x,
                        local_pos.y,
                        local_pos.z,
                        channel_index,
                    );
                }
            }
            block_pos = block_pos >> 1;
            voxel_pos = voxel_pos >> 1;
        }
        self.with_generator(|gen| gen.generate_single(pos, channel_index).as_raw())
            .unwrap_or(defval)
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
        generator: Option<&dyn VoxelGenerator>,
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
        let mut blocks_to_process_per_lod: Vec<Vec<Vector3i>> = (0..lod_count)
            .map(|i| {
                if i == 0 {
                    modified_lod0_blocks.to_vec()
                } else {
                    Vec::new()
                }
            })
            .collect();

        // LOD0 phase: clear needs_lodding and record updates.
        for &block_pos in &blocks_to_process_per_lod[0] {
            let Some(block) = self.lods[0].map.get_block_mut(block_pos) else {
                // C++ uses ERR_CONTINUE; we just skip the missing block.
                continue;
            };
            block.set_needs_lodding(false);
            if let Some(out) = out_updated_blocks.as_deref_mut() {
                out.push(BlockLocation {
                    position: block_pos,
                    lod_index: 0,
                });
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
                        if let Some(generator) = generator {
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
                    if let Some(generator) = generator {
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
                src_block
                    .voxels()
                    .downscale_to(dst_voxels, Vector3i::zero(), src_size, dst_offset);
            }
        }
    }

    pub fn pre_generate_box(
        &mut self,
        voxel_box: Box3i,
        generator: Option<&dyn VoxelGenerator>,
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
                if let Some(generator) = generator {
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

    /// Increases the reference count of every loaded block in `blocks_box` at
    /// `lod_index`, returning the positions of the missing (not-loaded) ones
    /// and optionally shallow copies of the found blocks / their positions.
    ///
    /// Ports `VoxelData::view_area`. The C++ method is used by mesh block
    /// tasks to pin blocks they will read while the mesher runs on a worker
    /// thread. `unview_area` is the matching release.
    pub fn view_area(
        &mut self,
        mut blocks_box: Box3i,
        lod_index: usize,
        missing_blocks: Option<&mut Vec<Vector3i>>,
        found_blocks_positions: Option<&mut Vec<Vector3i>>,
        found_blocks: Option<&mut Vec<VoxelDataBlock>>,
    ) {
        let bounds_in_blocks = self.bounds_in_voxels.downscaled(self.block_size() as i32);
        blocks_box = blocks_box.clipped(bounds_in_blocks);

        let Some(lod) = self.lods.get_mut(lod_index) else {
            return;
        };

        let mut missing_local = Vec::new();
        let mut found_positions_local = Vec::new();
        let mut found_blocks_local: Vec<VoxelDataBlock> = Vec::new();

        for bpos in blocks_box.iter_cells_zxy() {
            match lod.map.get_block_mut(bpos) {
                Some(block) => {
                    block.viewers.add();
                    if found_blocks.is_some() {
                        // Shallow copy: voxels are deep, but the C++ path also
                        // returns a full copy of the `VoxelDataBlock` value.
                        found_blocks_local.push(clone_block(block));
                    }
                    if found_blocks_positions.is_some() {
                        found_positions_local.push(bpos);
                    }
                }
                None => {
                    if missing_blocks.is_some() {
                        missing_local.push(bpos);
                    }
                }
            }
        }

        if let Some(out) = missing_blocks {
            out.extend(missing_local);
        }
        if let Some(out) = found_blocks_positions {
            out.extend(found_positions_local);
        }
        if let Some(out) = found_blocks {
            out.extend(found_blocks_local);
        }
    }

    /// Decreases the reference count of every loaded block in `blocks_box` at
    /// `lod_index`. Blocks reaching zero viewers are removed; if they were
    /// modified and `to_save` is provided, their voxels are returned for the
    /// caller to persist. Ports `VoxelData::unview_area`.
    pub fn unview_area(
        &mut self,
        mut blocks_box: Box3i,
        lod_index: usize,
        removed_blocks: Option<&mut Vec<Vector3i>>,
        missing_blocks: Option<&mut Vec<Vector3i>>,
        mut to_save: Option<&mut Vec<BlockToSave>>,
    ) {
        let bounds_in_blocks = self.bounds_in_voxels.downscaled(self.block_size() as i32);
        blocks_box = blocks_box.clipped(bounds_in_blocks);

        let Some(lod) = self.lods.get_mut(lod_index) else {
            // Still report every block as missing to mirror C++ behaviour.
            if let Some(out) = missing_blocks {
                out.extend(blocks_box.iter_cells_zxy());
            }
            return;
        };

        let mut removed_local = Vec::new();
        let mut missing_local = Vec::new();
        let saves_local: Vec<BlockToSave> = Vec::new();

        for bpos in blocks_box.iter_cells_zxy() {
            // Borrow, decrement, and decide whether to remove. We do this in
            // two steps because removing the block invalidates any outstanding
            // borrow of the map.
            let should_remove = match lod.map.get_block_mut(bpos) {
                Some(block) => {
                    block.viewers.remove();
                    block.viewers.get() == 0
                }
                None => {
                    missing_local.push(bpos);
                    continue;
                }
            };

            if should_remove {
                if let Some(block) = lod.map.remove_block(bpos) {
                    if let Some(out) = to_save.as_deref_mut() {
                        if block.is_modified() {
                            out.push(BlockToSave {
                                voxels: block.into_voxels(),
                                position: bpos,
                                lod_index: lod_index as u8,
                            });
                        }
                    }
                    removed_local.push(bpos);
                }
            }
        }

        if let Some(out) = removed_blocks {
            out.extend(removed_local);
        }
        if let Some(out) = missing_blocks {
            out.extend(missing_local);
        }
        if let Some(out) = to_save {
            out.extend(saves_local);
        }
    }
}

/// Copy a `VoxelDataBlock` for `view_area`'s found-blocks return. The C++
/// implementation returns a full value copy of the block; we do the same,
/// deep-copying the underlying `VoxelBuffer`. The refcount is also copied
/// (post-increment) so the snapshot reflects the live count.
fn clone_block(block: &VoxelDataBlock) -> VoxelDataBlock {
    let mut copy = match block.has_voxels() {
        true => VoxelDataBlock::with_voxels(block.voxels().copy_to_owned(), block.lod_index()),
        false => VoxelDataBlock::empty(block.lod_index()),
    };
    copy.set_modified(block.is_modified());
    copy.set_edited(block.is_edited());
    copy.set_needs_lodding(block.needs_lodding());
    copy.viewers = block.viewers;
    copy
}

#[cfg(test)]
mod tests {
    use super::{
        BlockLocation, SharedVoxelData, SharedVoxelDataEditPhase, SharedVoxelGenerator, VoxelData,
    };
    use crate::generators::base::{GenResult, VoxelGenerator, VoxelQueryData};
    use crate::math::{Box3i, Vector3i};
    use crate::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelDataBlock, VoxelFormat};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingGenerator {
        calls: Mutex<Vec<(Vector3i, u32)>>,
    }

    impl VoxelGenerator for RecordingGenerator {
        fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
            self.calls
                .lock()
                .unwrap()
                .push((input.origin_in_voxels, input.lod));
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
        let generator = RecordingGenerator::default();

        let generated = data.pre_generate_box(
            Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)),
            Some(&generator),
        );

        assert_eq!(generated, 3);
        assert_eq!(
            *generator.calls.lock().unwrap(),
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
        let generator = RecordingGenerator::default();

        let generated = data.pre_generate_box(
            Box3i::new(Vector3i::zero(), Vector3i::new(32, 16, 16)),
            Some(&generator),
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
        let generator = RecordingGenerator::default();
        data.update_lods(&modified, Some(&generator), None);

        // LOD1 block (0,0,0) was generated on demand and is now present.
        assert!(data.has_block(Vector3i::zero(), 1));
        assert!(generator
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(origin, lod)| { *lod == 1 && origin.x == 0 && origin.y == 0 && origin.z == 0 }));
    }

    #[test]
    fn view_area_increments_viewers_and_reports_found_and_missing_blocks() {
        let mut data = VoxelData::new();
        // Bounds cover a 4×4×4 block region so view queries can probe blocks
        // that exist alongside ones that don't, without being clipped out.
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(64)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();

        // Two loaded blocks; (2,0,0) is left empty within the queried area.
        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), channel));
        assert!(data.try_set_voxel(2, Vector3i::new(20, 1, 1), channel));

        let mut missing = Vec::new();
        let mut found_positions = Vec::new();
        let mut found_blocks: Vec<VoxelDataBlock> = Vec::new();
        data.view_area(
            Box3i::new(Vector3i::zero(), Vector3i::new(3, 1, 1)),
            0,
            Some(&mut missing),
            Some(&mut found_positions),
            Some(&mut found_blocks),
        );

        assert_eq!(
            found_positions,
            vec![Vector3i::zero(), Vector3i::new(1, 0, 0)]
        );
        assert_eq!(missing, vec![Vector3i::new(2, 0, 0)]);
        assert_eq!(found_blocks.len(), 2);
        // Viewers were incremented on the live blocks.
        assert_eq!(
            data.get_block(Vector3i::zero(), 0).unwrap().viewers.get(),
            1
        );
        assert_eq!(
            data.get_block(Vector3i::new(1, 0, 0), 0)
                .unwrap()
                .viewers
                .get(),
            1
        );
    }

    #[test]
    fn unview_area_releases_viewers_and_removes_blocks_reaching_zero() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();

        // Block A is unmodified; block B is modified and should be returned
        // for saving when it is unloaded by the unview.
        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), channel));
        assert!(data.try_set_voxel(2, Vector3i::new(20, 1, 1), channel));
        data.mark_area_modified(
            Box3i::new(Vector3i::new(16, 0, 0), Vector3i::new(32, 16, 16)),
            false,
        );

        // Pin both blocks, then release them.
        data.view_area(
            Box3i::new(Vector3i::zero(), Vector3i::new(2, 1, 1)),
            0,
            None,
            None,
            None,
        );
        let mut removed = Vec::new();
        let mut saves = Vec::new();
        data.unview_area(
            Box3i::new(Vector3i::zero(), Vector3i::new(2, 1, 1)),
            0,
            Some(&mut removed),
            None,
            Some(&mut saves),
        );

        assert_eq!(removed, vec![Vector3i::zero(), Vector3i::new(1, 0, 0)]);
        assert!(!data.has_block(Vector3i::zero(), 0));
        assert!(!data.has_block(Vector3i::new(1, 0, 0), 0));
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].position, Vector3i::new(1, 0, 0));
        assert!(saves[0].voxels.is_some());
    }

    #[test]
    fn unview_area_keeps_blocks_with_remaining_viewers() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();
        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), channel));

        // View the same block twice; a single unview should leave it pinned.
        data.view_area(
            Box3i::new(Vector3i::zero(), Vector3i::splat(1)),
            0,
            None,
            None,
            None,
        );
        data.view_area(
            Box3i::new(Vector3i::zero(), Vector3i::splat(1)),
            0,
            None,
            None,
            None,
        );
        assert_eq!(
            data.get_block(Vector3i::zero(), 0).unwrap().viewers.get(),
            2
        );

        let mut removed = Vec::new();
        data.unview_area(
            Box3i::new(Vector3i::zero(), Vector3i::splat(1)),
            0,
            Some(&mut removed),
            None,
            None,
        );

        assert!(removed.is_empty());
        assert!(data.has_block(Vector3i::zero(), 0));
        assert_eq!(
            data.get_block(Vector3i::zero(), 0).unwrap().viewers.get(),
            1
        );
    }

    #[test]
    fn set_generator_attaches_a_shared_handle_round_trippable_via_with_generator() {
        let mut data = VoxelData::new();
        assert!(!data.has_generator());

        let generator: SharedVoxelGenerator = Arc::new(RecordingGenerator::default());
        data.set_generator(Some(generator.clone()));
        assert!(data.has_generator());

        let mut probed = Vec::new();
        data.with_generator(|gen| {
            // Touch the generator under its lock; the recorder stores nothing
            // externally but we can confirm it runs.
            let _ = gen.used_channels_mask();
            probed.push(());
        });
        assert_eq!(probed.len(), 1);
        assert!(Arc::ptr_eq(data.generator().as_ref().unwrap(), &generator));
    }

    #[test]
    fn shared_voxel_data_region_locks_follow_voxel_data_contract() {
        let shared = SharedVoxelData::new(VoxelData::new());
        let area = Box3i::new(Vector3i::zero(), Vector3i::splat(16));
        let overlap = Box3i::new(Vector3i::splat(8), Vector3i::splat(16));
        let disjoint = Box3i::new(Vector3i::splat(64), Vector3i::splat(16));

        let read = shared.read_region(0, area);
        let overlap_read = shared
            .try_read_region(0, overlap)
            .expect("overlapping mesh/read regions may coexist");
        assert!(
            shared.try_write_region(0, overlap).is_none(),
            "overlapping edit/write region must wait for readers"
        );
        let disjoint_write = shared
            .try_write_region(0, disjoint)
            .expect("disjoint edit/write region can proceed");
        assert_eq!(shared.locked_region_count(0), 3);

        drop(disjoint_write);
        drop(overlap_read);
        drop(read);

        let write = shared
            .try_write_region(0, overlap)
            .expect("write should acquire after readers drop");
        assert_eq!(shared.locked_region_count(0), 1);
        drop(write);
        assert_eq!(shared.locked_region_count(0), 0);
    }

    #[test]
    fn shared_edit_voxel_materializes_procedural_block_and_marks_it_dirty() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        data.set_generator(Some(Arc::new(RecordingGenerator::default())));
        let shared = SharedVoxelData::new(data);
        let channel = ChannelId::Type.index();

        assert!(shared.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel));

        let block = shared.block_snapshot(Vector3i::zero(), 0).unwrap();
        assert_eq!(block.voxels().get_voxel(1, 1, 1, channel), 99);
        assert_eq!(block.voxels().get_voxel(2, 1, 1, channel), 10);
        assert!(block.is_modified());
        assert!(block.is_edited());
    }

    #[test]
    fn shared_edit_voxel_does_not_materialize_unavailable_blocks() {
        let channel = ChannelId::Type.index();

        for &(streaming_enabled, full_load_completed) in &[(true, true), (false, false)] {
            for empty_block in [false, true] {
                let mut data = VoxelData::new();
                data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
                data.set_streaming_enabled(streaming_enabled);
                data.set_full_load_completed(full_load_completed);
                let generator = Arc::new(RecordingGenerator::default());
                data.set_generator(Some(generator.clone()));
                let shared = SharedVoxelData::new(data);

                if empty_block {
                    assert!(shared.try_set_block(Vector3i::zero(), VoxelDataBlock::empty(0)));
                }

                assert!(
                    !shared.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel),
                    "streaming={streaming_enabled}, full_load_completed={full_load_completed}, empty_block={empty_block}"
                );
                assert!(generator.calls.lock().unwrap().is_empty());

                match shared.block_snapshot(Vector3i::zero(), 0) {
                    Some(block) if empty_block => assert!(!block.has_voxels()),
                    None if !empty_block => {}
                    _ => panic!(
                        "unavailable block was materialized: streaming={streaming_enabled}, full_load_completed={full_load_completed}, empty_block={empty_block}"
                    ),
                }
            }
        }
    }

    struct SpatialLockProbeGenerator {
        data: std::sync::Weak<SharedVoxelData>,
    }

    impl VoxelGenerator for SpatialLockProbeGenerator {
        fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
            let data = self
                .data
                .upgrade()
                .expect("shared data survives generation");
            let block_box = Box3i::new(input.origin_in_voxels, input.buffer.size());
            assert!(
                data.try_write_region(0, block_box).is_none(),
                "try_edit_voxel must hold the target spatial write region during generation"
            );
            input.buffer.fill(42, ChannelId::Type.index());
            GenResult::default()
        }
    }

    #[test]
    fn shared_edit_voxel_holds_write_region_during_generator_materialization() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let shared = Arc::new(SharedVoxelData::new(data));
        shared.set_generator(Some(Arc::new(SpatialLockProbeGenerator {
            data: Arc::downgrade(&shared),
        })));
        let channel = ChannelId::Type.index();

        assert!(shared.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel));
        assert_eq!(
            shared
                .block_snapshot(Vector3i::zero(), 0)
                .unwrap()
                .voxels()
                .get_voxel(2, 1, 1, channel),
            42
        );
    }

    #[test]
    fn shared_edit_voxel_signals_spatial_lock_before_waiting_for_map_lock() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let shared = Arc::new(SharedVoxelData::new(data));
        let spatial_phase = Arc::new((Mutex::new(false), Condvar::new()));
        shared.set_test_edit_phase_hook(Arc::new({
            let spatial_phase = spatial_phase.clone();
            move |phase| {
                if phase == SharedVoxelDataEditPhase::SpatialWriteAcquiredBeforeMapLock {
                    let (lock, cvar) = &*spatial_phase;
                    *lock.lock().unwrap() = true;
                    cvar.notify_one();
                }
            }
        }));

        let held_map = shared
            .try_lod_map_write(0)
            .expect("test must hold the LOD map write lock");
        let edit_data = shared.clone();
        let channel = ChannelId::Type.index();
        let edit = std::thread::spawn(move || {
            edit_data.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel)
        });

        let (phase_lock, phase_cvar) = &*spatial_phase;
        let mut signalled = phase_lock.lock().unwrap();
        let reached_before_map_unlock = loop {
            if *signalled {
                break true;
            }
            let (next, timeout) = phase_cvar
                .wait_timeout(signalled, Duration::from_secs(1))
                .unwrap();
            signalled = next;
            if timeout.timed_out() && !*signalled {
                break false;
            }
        };
        let spatial_lock_held = reached_before_map_unlock
            && shared
                .try_write_region(0, Box3i::new(Vector3i::zero(), Vector3i::splat(16)))
                .is_none();
        drop(signalled);
        drop(held_map);

        assert!(edit.join().unwrap());
        assert!(
            reached_before_map_unlock,
            "try_edit_voxel did not signal the spatial phase before the blocked map lock"
        );
        assert!(
            spatial_lock_held,
            "the target spatial write region was not held while the spatial phase was signalled"
        );
    }

    #[test]
    fn shared_edit_voxel_keeps_map_write_lock_until_dirty_flags_are_set() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let shared = Arc::new(SharedVoxelData::new(data));
        assert!(shared.try_set_block(
            Vector3i::zero(),
            VoxelDataBlock::with_voxels(VoxelBuffer::with_size(Vector3i::splat(16)), 0),
        ));
        let before_dirty = Arc::new((Mutex::new(false), Condvar::new()));
        let release_before_dirty = Arc::new((Mutex::new(false), Condvar::new()));
        let after_dirty = Arc::new((Mutex::new(false), Condvar::new()));
        let release_after_dirty = Arc::new((Mutex::new(false), Condvar::new()));
        shared.set_test_edit_phase_hook(Arc::new({
            let before_dirty = before_dirty.clone();
            let release_before_dirty = release_before_dirty.clone();
            let after_dirty = after_dirty.clone();
            let release_after_dirty = release_after_dirty.clone();
            move |phase| match phase {
                SharedVoxelDataEditPhase::VoxelWrittenBeforeDirtyFlags => {
                    let (entered_lock, entered_cvar) = &*before_dirty;
                    *entered_lock.lock().unwrap() = true;
                    entered_cvar.notify_one();

                    let (release_lock, release_cvar) = &*release_before_dirty;
                    let mut released = release_lock.lock().unwrap();
                    while !*released {
                        let (next, timeout) = release_cvar
                            .wait_timeout(released, Duration::from_secs(1))
                            .unwrap();
                        released = next;
                        assert!(
                            !timeout.timed_out(),
                            "edit phase timed out before dirty flags; map write lock may have escaped its closure"
                        );
                    }
                }
                SharedVoxelDataEditPhase::DirtyFlagsSetBeforeMapWriteUnlock => {
                    let (entered_lock, entered_cvar) = &*after_dirty;
                    *entered_lock.lock().unwrap() = true;
                    entered_cvar.notify_one();

                    let (release_lock, release_cvar) = &*release_after_dirty;
                    let mut released = release_lock.lock().unwrap();
                    while !*released {
                        let (next, timeout) = release_cvar
                            .wait_timeout(released, Duration::from_secs(1))
                            .unwrap();
                        released = next;
                        assert!(
                            !timeout.timed_out(),
                            "edit phase timed out after dirty flags; map write lock may have escaped its closure"
                        );
                    }
                }
                SharedVoxelDataEditPhase::SpatialWriteAcquiredBeforeMapLock => {}
            }
        }));

        let edit_data = shared.clone();
        let channel = ChannelId::Type.index();
        let edit = std::thread::spawn(move || {
            edit_data.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel)
        });

        let (entered_lock, entered_cvar) = &*before_dirty;
        let mut entered = entered_lock.lock().unwrap();
        let reached_before_dirty = loop {
            if *entered {
                break true;
            }
            let (next, timeout) = entered_cvar
                .wait_timeout(entered, Duration::from_secs(1))
                .unwrap();
            entered = next;
            if timeout.timed_out() && !*entered {
                break false;
            }
        };
        let map_still_write_locked = reached_before_dirty && shared.try_lod_map_read(0).is_none();
        drop(entered);
        let (release_lock, release_cvar) = &*release_before_dirty;
        *release_lock.lock().unwrap() = true;
        release_cvar.notify_one();

        let (after_dirty_lock, after_dirty_cvar) = &*after_dirty;
        let mut after_dirty_entered = after_dirty_lock.lock().unwrap();
        let reached_after_dirty = loop {
            if *after_dirty_entered {
                break true;
            }
            let (next, timeout) = after_dirty_cvar
                .wait_timeout(after_dirty_entered, Duration::from_secs(1))
                .unwrap();
            after_dirty_entered = next;
            if timeout.timed_out() && !*after_dirty_entered {
                break false;
            }
        };
        let map_still_write_locked_after_dirty =
            reached_after_dirty && shared.try_lod_map_read(0).is_none();
        drop(after_dirty_entered);
        let (release_lock, release_cvar) = &*release_after_dirty;
        *release_lock.lock().unwrap() = true;
        release_cvar.notify_one();

        assert!(edit.join().unwrap());
        assert!(
            reached_before_dirty,
            "try_edit_voxel did not expose the pre-dirty phase"
        );
        assert!(
            map_still_write_locked,
            "try_edit_voxel released its map write lock between voxel mutation and dirty flags"
        );
        assert!(
            reached_after_dirty,
            "try_edit_voxel did not expose the fully dirty phase"
        );
        assert!(
            map_still_write_locked_after_dirty,
            "try_edit_voxel released its map write lock after dirty flags but before leaving the map write closure"
        );
    }

    #[test]
    fn shared_edit_voxel_is_dirty_before_immediate_unview() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let shared = SharedVoxelData::new(data);
        let channel = ChannelId::Type.index();

        assert!(shared.try_edit_voxel(77, Vector3i::new(1, 1, 1), channel));
        let mut saves = Vec::new();
        let area = Box3i::new(Vector3i::zero(), Vector3i::splat(1));
        let voxel_area = Box3i::new(Vector3i::zero(), Vector3i::splat(16));
        let _region = shared.write_region(0, voxel_area);
        shared.unview_area(area, 0, None, None, Some(&mut saves));

        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].position, Vector3i::zero());
        assert_eq!(
            saves[0]
                .voxels
                .as_ref()
                .unwrap()
                .get_voxel(1, 1, 1, channel),
            77
        );
    }

    struct BlockingGenerator {
        entered: Arc<(Mutex<bool>, Condvar)>,
        release: Arc<(Mutex<bool>, Condvar)>,
        resident_inserted: Arc<(Mutex<bool>, Condvar)>,
    }

    impl VoxelGenerator for BlockingGenerator {
        fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
            let (entered_lock, entered_cvar) = &*self.entered;
            *entered_lock.lock().unwrap() = true;
            entered_cvar.notify_one();
            let (release_lock, release_cvar) = &*self.release;
            let mut released = release_lock.lock().unwrap();
            while !*released {
                let (next, timeout) = release_cvar
                    .wait_timeout(released, Duration::from_secs(1))
                    .unwrap();
                released = next;
                assert!(
                    !timeout.timed_out(),
                    "blocking generator timed out waiting for release"
                );
            }
            drop(released);

            let (resident_lock, resident_cvar) = &*self.resident_inserted;
            let mut inserted = resident_lock.lock().unwrap();
            while !*inserted {
                let (next, timeout) = resident_cvar
                    .wait_timeout(inserted, Duration::from_secs(1))
                    .unwrap();
                inserted = next;
                assert!(
                    !timeout.timed_out(),
                    "blocking generator timed out waiting for resident insertion; map lock may be held during generation"
                );
            }
            input.buffer.fill(10, ChannelId::Type.index());
            GenResult::default()
        }
    }

    #[test]
    fn shared_edit_voxel_keeps_resident_block_inserted_during_materialization() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let entered = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let resident_inserted = Arc::new((Mutex::new(false), Condvar::new()));
        data.set_generator(Some(Arc::new(BlockingGenerator {
            entered: entered.clone(),
            release: release.clone(),
            resident_inserted: resident_inserted.clone(),
        })));
        let shared = Arc::new(SharedVoxelData::new(data));
        let channel = ChannelId::Type.index();
        let edit_data = shared.clone();
        let edit = std::thread::spawn(move || {
            edit_data.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel)
        });

        let (entered_lock, entered_cvar) = &*entered;
        let mut started = entered_lock.lock().unwrap();
        while !*started {
            let (next, timeout) = entered_cvar
                .wait_timeout(started, Duration::from_secs(1))
                .unwrap();
            started = next;
            assert!(
                !timeout.timed_out(),
                "try_edit_voxel never entered procedural materialization"
            );
        }
        drop(started);
        let (release_lock, release_cvar) = &*release;
        *release_lock.lock().unwrap() = true;
        release_cvar.notify_one();
        let mut resident = VoxelBuffer::with_size(Vector3i::splat(16));
        resident.set_voxel(33, 2, 1, 1, channel);
        assert!(shared.try_set_block(Vector3i::zero(), VoxelDataBlock::with_voxels(resident, 0)));
        let (resident_lock, resident_cvar) = &*resident_inserted;
        *resident_lock.lock().unwrap() = true;
        resident_cvar.notify_one();
        assert!(edit.join().unwrap());

        let block = shared.block_snapshot(Vector3i::zero(), 0).unwrap();
        assert_eq!(block.voxels().get_voxel(1, 1, 1, channel), 99);
        assert_eq!(block.voxels().get_voxel(2, 1, 1, channel), 33);
    }

    struct MapUnlockProbeGenerator {
        data: std::sync::Weak<SharedVoxelData>,
    }

    impl VoxelGenerator for MapUnlockProbeGenerator {
        fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
            let data = self
                .data
                .upgrade()
                .expect("shared data survives generation");
            drop(
                data.try_lod_map_write(0)
                    .expect("generator must run without map write lock"),
            );
            input.buffer.fill(42, ChannelId::Type.index());
            GenResult::default()
        }
    }

    #[test]
    fn shared_edit_voxel_runs_generator_without_map_lock() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let shared = Arc::new(SharedVoxelData::new(data));
        shared.set_generator(Some(Arc::new(MapUnlockProbeGenerator {
            data: Arc::downgrade(&shared),
        })));
        let channel = ChannelId::Type.index();

        assert!(shared.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel));
        assert_eq!(
            shared
                .block_snapshot(Vector3i::zero(), 0)
                .unwrap()
                .voxels()
                .get_voxel(2, 1, 1, channel),
            42
        );
    }

    #[test]
    fn shared_voxel_data_allows_parallel_read_snapshots() {
        let shared = Arc::new(SharedVoxelData::new(VoxelData::new()));
        let entered = Arc::new((Mutex::new(0usize), Condvar::new()));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let shared = shared.clone();
                let entered = entered.clone();
                std::thread::spawn(move || {
                    shared.with_settings(|_| {
                        let (lock, cvar) = &*entered;
                        let mut count = lock.lock().unwrap();
                        *count += 1;
                        cvar.notify_all();
                        while *count < 2 {
                            let (next, timeout) =
                                cvar.wait_timeout(count, Duration::from_secs(1)).unwrap();
                            count = next;
                            if timeout.timed_out() && *count < 2 {
                                return false;
                            }
                        }
                        true
                    })
                })
            })
            .collect();

        for handle in handles {
            assert!(
                handle.join().unwrap(),
                "SharedVoxelData read snapshots should overlap"
            );
        }
    }

    #[test]
    fn shared_voxel_data_allows_parallel_lod_map_writes() {
        let mut data = VoxelData::new();
        data.set_lod_count(2);
        let shared = Arc::new(SharedVoxelData::new(data));
        let entered = Arc::new((Mutex::new(0usize), Condvar::new()));

        let handles: Vec<_> = (0..2)
            .map(|lod_index| {
                let shared = shared.clone();
                let entered = entered.clone();
                std::thread::spawn(move || {
                    shared.with_lod_map_mut(lod_index, |_| {
                        let (lock, cvar) = &*entered;
                        let mut count = lock.lock().unwrap();
                        *count += 1;
                        cvar.notify_all();
                        while *count < 2 {
                            let (next, timeout) =
                                cvar.wait_timeout(count, Duration::from_secs(1)).unwrap();
                            count = next;
                            if timeout.timed_out() && *count < 2 {
                                return false;
                            }
                        }
                        true
                    })
                })
            })
            .collect();

        for handle in handles {
            assert!(
                handle.join().unwrap(),
                "SharedVoxelData writes to different LOD maps should overlap"
            );
        }
    }

    #[test]
    fn copy_round_trips_through_lod0_with_generator_filling_missing_blocks() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        let channel = ChannelId::Type.index();
        let generator: SharedVoxelGenerator = Arc::new(RecordingGenerator::default());
        data.set_generator(Some(generator));

        // No blocks loaded yet. Copy must invoke the generator for the area.
        let mut dst = VoxelBuffer::with_size(Vector3i::new(16, 16, 16));
        data.copy(Vector3i::zero(), &mut dst, 1u32 << channel, true);

        // RecordingGenerator writes `10 + lod + origin.x`; for block (0,0,0)
        // and lod 0 that is 10. The generator is invoked once per block here.
        assert_eq!(dst.get_voxel(0, 0, 0, channel), 10);
    }

    #[test]
    fn copy_without_generator_returns_defaults_for_missing_blocks() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        let channel = ChannelId::Type.index();
        let mut dst = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        data.copy(Vector3i::zero(), &mut dst, 1u32 << channel, true);

        // No generator and no blocks: dst stays at default (0 for Type).
        assert_eq!(dst.get_voxel(0, 0, 0, channel), 0);
    }

    #[test]
    fn paste_and_paste_masked_route_into_lod0_map() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();
        let mask = 1u32 << channel;

        let mut source = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        source.fill(7, channel);
        data.paste(Vector3i::zero(), &source, mask, true);
        assert_eq!(data.get_voxel(Vector3i::new(1, 1, 1), channel, 0), 7);

        // Masked paste: skip voxels equal to the mask sentinel.
        let mut masked_source = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        masked_source.fill(9, channel);
        data.paste_masked(
            Vector3i::zero(),
            &masked_source,
            mask,
            channel,
            9, // skip everything → no writes
            true,
        );
        // Unchanged because every source voxel matched the mask sentinel.
        assert_eq!(data.get_voxel(Vector3i::zero(), channel, 0), 7);
    }

    #[test]
    fn is_area_loaded_reflects_block_residency_and_streaming_bounds() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(32)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);

        let area = Box3i::new(Vector3i::zero(), Vector3i::splat(16));
        assert!(!data.is_area_loaded(area));

        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), ChannelId::Type.index()));
        assert!(data.is_area_loaded(area));

        // Streaming-mode short-circuit: area outside bounds returns false.
        data.set_streaming_enabled(true);
        assert!(!data.is_area_loaded(Box3i::new(Vector3i::new(100, 0, 0), Vector3i::splat(16),)));
    }

    #[test]
    fn has_all_blocks_and_get_missing_blocks_agree() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(64)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), ChannelId::Type.index()));
        // Block (1,0,0) is intentionally left empty.

        let area = Box3i::new(Vector3i::zero(), Vector3i::new(2, 1, 1));
        assert!(!data.has_all_blocks_in_area(area, 0));

        let mut missing = Vec::new();
        data.get_missing_blocks(area, 0, &mut missing);
        assert_eq!(missing, vec![Vector3i::new(1, 0, 0)]);

        assert!(data.try_set_voxel(2, Vector3i::new(20, 1, 1), ChannelId::Type.index()));
        assert!(data.has_all_blocks_in_area(area, 0));
    }

    #[test]
    fn get_blocks_with_voxel_data_returns_grid_with_empty_slots_for_missing() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(64)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        assert!(data.try_set_voxel(1, Vector3i::new(1, 1, 1), ChannelId::Type.index()));
        // Add an empty (no-voxels) block alongside.
        assert!(data.try_set_block(Vector3i::new(1, 0, 0), VoxelDataBlock::empty(0)));

        let blocks = data
            .get_blocks_with_voxel_data(Box3i::new(Vector3i::zero(), Vector3i::new(2, 1, 1)), 0);
        // ZXY layout: (0,0,0) is index 0, (1,0,0) is index 1.
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].is_some());
        assert!(blocks[1].is_none()); // empty block has no voxel data
    }

    #[test]
    fn get_voxel_falls_back_to_generator_when_block_is_missing_non_streaming() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(64)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let channel = ChannelId::Type.index();

        // RecordingGenerator writes `10 + lod + origin.x`. The default
        // `generate_single` impl passes the queried voxel position as the
        // 1×1×1 block's origin, so for voxel (20,5,5) the result is 10+0+20=30.
        let generator: SharedVoxelGenerator = Arc::new(RecordingGenerator::default());
        data.set_generator(Some(generator));

        let value = data.get_voxel(Vector3i::new(20, 5, 5), channel, 0);
        assert_eq!(value, 30);
    }

    #[test]
    fn get_voxel_returns_defval_when_no_generator_and_block_missing_non_streaming() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(64)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);

        // No generator: the fallback returns the caller-provided default.
        assert_eq!(
            data.get_voxel(Vector3i::new(20, 5, 5), ChannelId::Type.index(), 99),
            99
        );
    }
}
