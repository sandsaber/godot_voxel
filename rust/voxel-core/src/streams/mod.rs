//! `streams` — serialization formats for voxel block persistence.
//!
//! Each submodule handles one of godot_voxel's internal on-disk / in-memory
//! block representations. These are the formats the terrain layer round-trips
//! through its block cache; they are **not** public asset formats like the
//! `.vox` parser in [`crate::format::vox`].
//!
//! ## Current
//! - [`block_serializer`] — `VoxelBuffer` ↔ bytes (v4 format), with optional
//!   LZ4/ZSTD compression. Depends on [`compressed_data`].
//! - [`compressed_data`] — LZ4/ZSTD compression envelope used by the block
//!   serializer. LZ4 is pure-Rust (`lz4_flex`); ZSTD is behind a feature.
//! - [`instance_data`] — lossy-compressed per-block instance transforms
//!   (instanced grass / detail).
//! - [`region`] — region-file archive format (`.vxr`): sector-based sparse
//!   block storage with header LUT, built on [`block_serializer`].
//! - [`stream_cache`] — in-memory `(position, lod)` → `VoxelBuffer` cache
//!   (`BlockCache`), ported from `voxel_stream_cache`. Single-threaded; the
//!   C++ per-LoD `RWLock` is omitted (Phase 4).
//! - [`voxel_stream`] — engine-agnostic base stream contract ported from
//!   `streams/voxel_stream`.
//! - [`stream_memory`] — fake in-memory `VoxelStream` for tests (`MemoryStream`),
//!   ported from `voxel_stream_memory`.

pub mod block_serializer;
pub mod compressed_data;
pub mod instance_data;
pub mod region;
pub mod save_block_data_task;
pub mod stream_cache;
pub mod stream_memory;
pub mod voxel_stream;

pub use save_block_data_task::SaveBlockDataTask;
pub use stream_cache::BlockCache;
pub use stream_memory::MemoryStream;
pub use voxel_stream::{
    LoadResult, SaveMode, StreamResult, VoxelLoadQuery, VoxelSaveQuery, VoxelStream,
    VoxelStreamError,
};
