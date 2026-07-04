//! `streams::stream_memory` — "fake" in-memory voxel stream for testing.
//!
//! Ported from `streams/voxel_stream_memory.{h,cpp}`. An in-memory stand-in for
//! a real [`VoxelStream`]: it stores blocks in a map keyed on `(position, lod)`
//! and round-trips them through `save_block` / `load_block` instead of touching
//! the filesystem. The C++ class exists almost entirely for unit tests, and the
//! Rust port keeps that role.
//!
//! ## What changed from C++
//!
//! The C++ `VoxelStreamMemory` inherits `VoxelStream` (a Godot `Resource`,
//! which drags in `ClassDB`, `GDCLASS`, `_bind_methods`, `Mutex` per LoD, an
//! `artificial_save_latency_usec` knob, batched `Span<VoxelQueryData>` entry
//! points, and the instance-block / load-all-blocks overrides). All of that
//! engine machinery is omitted here: the [`VoxelStream`] trait itself lands in
//! Phase 4 alongside the locking strategy, and the latency knob, batched API
//! and instance blocks are out of scope for this port. What remains is the data
//! storage — the part tests actually exercise.
//!
//! The C++ storage is `FixedArray<Lod, MAX_LOD>` with one
//! `StdUnorderedMap<Vector3i, VoxelChunk>` per LoD, each `VoxelChunk` wrapping
//! a `VoxelBuffer`. We collapse that to a flat
//! [`HashMap`]`<(Vector3i, u8), VoxelBuffer>` keyed on `(position, lod)`, which
//! is the same shape the sibling [`BlockCache`](crate::streams::BlockCache)
//! uses; the per-LoD split only existed to shard the `Mutex`.

use crate::math::Vector3i;
use crate::storage::VoxelBuffer;
use std::collections::HashMap;

/// Outcome of a [`MemoryStream::load_block`] attempt. Mirrors the relevant
/// subset of the C++ `VoxelStream::ResultCode` enum (only the two values a
/// memory stream can actually return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadResult {
    /// The block was found and copied into the caller's buffer. Corresponds to
    /// `RESULT_BLOCK_FOUND`.
    Found,
    /// No block is stored at the queried `(position, lod)`. Corresponds to
    /// `RESULT_BLOCK_NOT_FOUND`.
    NotFound,
}

/// Persistence capability reported by a stream. Mirrors the concept behind the
/// C++ `VoxelStream` save/load contract: a stream may be read/write, read-only
/// or non-persistent. The memory stream is read/write but its data lives only
/// in RAM, hence [`SaveMode::Memory`].
///
/// This stands in for the spec's `get_supported_save_mode`: the underlying C++
/// class does not declare that exact method (its closest kin are
/// `supports_loading_all_blocks` and `get_used_channels_mask`), but a save-mode
/// flag is what a `MemoryStream` test harness needs to pick a persistence
/// strategy, so we expose it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaveMode {
    /// The stream cannot persist blocks at all (a pure generator / read-only
    /// source). Saves are dropped on the floor.
    #[default]
    None,
    /// Blocks persist for the lifetime of the process — i.e. in memory. This is
    /// what [`MemoryStream`] reports.
    Memory,
}

/// In-memory voxel stream: stores block copies in a `HashMap`, never touching
/// the filesystem. Ported from `VoxelStreamMemory` (data-storage half only).
///
/// The stream owns every stored [`VoxelBuffer`]. [`MemoryStream::save_block`]
/// copies the supplied buffer in (matching the C++ `copy_to`), and
/// [`MemoryStream::load_block`] hands back a fresh copy on a hit — the C++
/// memory stream likewise retains ownership and the caller's buffer is
/// populated by copy.
#[derive(Debug, Default)]
pub struct MemoryStream {
    blocks: HashMap<(Vector3i, u8), VoxelBuffer>,
}

impl MemoryStream {
    /// Empty stream. Matches a default-constructed `VoxelStreamMemory`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of blocks currently stored. The C++ class has no exact
    /// equivalent (it exposes `load_all_blocks` instead), but the count is the
    /// natural invariant for tests.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the stream holds any blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Whether the stream reports the given persistence mode. Always
    /// [`SaveMode::Memory`] for [`MemoryStream`]; provided so a generic test
    /// harness can branch on it the way it would on `VoxelStream::supports_*`.
    pub fn get_supported_save_mode(&self) -> SaveMode {
        SaveMode::Memory
    }

    /// Store a copy of `voxels` at `(position, lod)`, overwriting any prior
    /// block at that key. Ported from `save_voxel_blocks` (single-block path).
    ///
    /// The C++ path copies via `voxel_buffer.copy_to(dst.voxels, true)`; we do
    /// the same, cloning the source buffer into the map so the caller keeps
    /// its own copy. Saving a block whose size is empty is silently ignored,
    /// matching the "you can't have meaningfully saved nothing" invariant the
    /// cache enforces too.
    pub fn save_block(&mut self, position: Vector3i, lod: u8, voxels: &VoxelBuffer) {
        if voxels.size().is_empty_size() {
            return;
        }
        let mut entry = VoxelBuffer::new(voxels.allocator());
        copy_buffer_into(voxels, &mut entry);
        self.blocks.insert((position, lod), entry);
    }

    /// Load the block at `(position, lod)` into `out_voxels`. Ported from
    /// `load_voxel_blocks` (single-block path).
    ///
    /// Returns [`LoadResult::Found`] and copies the stored buffer into
    /// `out_voxels` (resized to match) on a hit; [`LoadResult::NotFound`] on a
    /// miss, leaving `out_voxels` untouched — exactly as the C++ code leaves
    /// `q.voxel_buffer` untouched when setting `RESULT_BLOCK_NOT_FOUND`.
    pub fn load_block(
        &self,
        position: Vector3i,
        lod: u8,
        out_voxels: &mut VoxelBuffer,
    ) -> LoadResult {
        let Some(stored) = self.blocks.get(&(position, lod)) else {
            return LoadResult::NotFound;
        };
        copy_buffer_into(stored, out_voxels);
        LoadResult::Found
    }

    /// Remove a stored block. Useful when a test wants to model a block being
    /// deleted between a save and a subsequent load. No direct C++ counterpart
    /// (the memory stream never erases), but trivially faithful to the
    /// underlying map storage.
    pub fn remove(&mut self, position: Vector3i, lod: u8) -> bool {
        self.blocks.remove(&(position, lod)).is_some()
    }

    /// Drop every stored block.
    pub fn clear(&mut self) {
        self.blocks.clear();
    }
}

/// Deep-copy `src` into `dst`, resizing `dst` and replicating every channel's
/// depth / compression / data. Stands in for the C++ `VoxelBuffer::copy_to(dst,
/// true)` used by the memory stream on both save and load. `dst` keeps its own
/// allocator/pool, matching the C++ contract.
fn copy_buffer_into(src: &VoxelBuffer, dst: &mut VoxelBuffer) {
    dst.create(src.size());
    // Mirrors `copy_to(_, /*copy_channels=*/true)`: copies depth, compression,
    // default value and raw bytes for all eight channels.
    dst.copy_channels_from(src);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vector3i;
    use crate::storage::{Allocator, ChannelId, VoxelBuffer};

    /// Build a small non-uniform block: a 2³ buffer with `value` written to one
    /// voxel in the Type channel, so a copy is observably different from a
    /// freshly-created uniform-default buffer.
    fn sample_block(value: u64) -> VoxelBuffer {
        let mut b = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        b.set_voxel(value, 1, 0, 1, ChannelId::Type.index());
        b
    }

    /// Read the Type-channel voxel back, exercising the (de)compress path so
    /// stored copies stay byte-identical with the original.
    fn type_voxel(buf: &VoxelBuffer, x: i32, y: i32, z: i32) -> u64 {
        buf.get_voxel(x, y, z, ChannelId::Type.index())
    }

    #[test]
    fn supported_save_mode_is_memory() {
        let stream = MemoryStream::new();
        assert_eq!(stream.get_supported_save_mode(), SaveMode::Memory);
    }

    #[test]
    fn save_then_load_round_trips_block_data() {
        let mut stream = MemoryStream::new();
        let pos = Vector3i::new(5, -3, 1);
        let stored = sample_block(123);

        stream.save_block(pos, 0, &stored);
        assert_eq!(stream.len(), 1);

        let mut loaded = VoxelBuffer::new(Allocator::Default);
        assert_eq!(stream.load_block(pos, 0, &mut loaded), LoadResult::Found);
        assert_eq!(loaded.size(), Vector3i::new(2, 2, 2));
        assert_eq!(type_voxel(&loaded, 1, 0, 1), 123);
        // Untouched voxels keep the (now-materialized) default of 0.
        assert_eq!(type_voxel(&loaded, 0, 0, 0), 0);
    }

    #[test]
    fn load_returns_not_found_on_miss_and_leaves_buffer_untouched() {
        let stream = MemoryStream::new();
        let mut out = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        out.set_voxel(7, 0, 0, 0, ChannelId::Type.index());

        assert_eq!(
            stream.load_block(Vector3i::new(0, 0, 0), 0, &mut out),
            LoadResult::NotFound
        );
        // Miss must not touch the caller's buffer.
        assert_eq!(type_voxel(&out, 0, 0, 0), 7);
    }

    #[test]
    fn save_overwrites_existing_block_at_same_key() {
        let mut stream = MemoryStream::new();
        let pos = Vector3i::new(2, 2, 2);

        stream.save_block(pos, 0, &sample_block(1));
        stream.save_block(pos, 0, &sample_block(2));
        assert_eq!(stream.len(), 1, "overwrite must not grow the entry count");

        let mut loaded = VoxelBuffer::new(Allocator::Default);
        assert_eq!(stream.load_block(pos, 0, &mut loaded), LoadResult::Found);
        assert_eq!(type_voxel(&loaded, 1, 0, 1), 2);
    }

    #[test]
    fn save_ignores_empty_size_buffer() {
        let mut stream = MemoryStream::new();
        let empty = VoxelBuffer::new(Allocator::Default); // size (0,0,0)
        stream.save_block(Vector3i::new(0, 0, 0), 0, &empty);
        assert!(stream.is_empty(), "empty-size buffer must not be stored");
    }

    #[test]
    fn keys_distinct_on_position_and_lod() {
        let mut stream = MemoryStream::new();
        let pos = Vector3i::new(1, 1, 1);

        stream.save_block(pos, 0, &sample_block(10));
        stream.save_block(pos, 2, &sample_block(20));
        assert_eq!(stream.len(), 2);

        let mut a = VoxelBuffer::new(Allocator::Default);
        let mut b = VoxelBuffer::new(Allocator::Default);
        assert_eq!(stream.load_block(pos, 0, &mut a), LoadResult::Found);
        assert_eq!(stream.load_block(pos, 2, &mut b), LoadResult::Found);
        assert_eq!(type_voxel(&a, 1, 0, 1), 10);
        assert_eq!(type_voxel(&b, 1, 0, 1), 20);
    }
}
