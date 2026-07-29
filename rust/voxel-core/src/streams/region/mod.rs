//! `streams::region` — godot_voxel's region-file archive format.
//!
//! Ported from `streams/region/region_file.{h,cpp}`. A region file (`*.vxr`)
//! stores up to `region_size³` voxel blocks in a sector-based sparse layout.
//! Each block is a length-prefixed [`crate::streams::block_serializer`] payload
//! (optionally LZ4/ZSTD-compressed) padded to a sector boundary; a header LUT
//! maps block positions to sector ranges.
//!
//! ## Current
//! - [`format`] — `RegionFormat`, `RegionBlockInfo`, on-disk constants.
//! - [`region_file`] — [`region_file::RegionFile`] with header save/load,
//!   sector allocation, `load_block`/`save_block`.
//!
//! ## Deferred
//! - **Forest wrapper** (`VoxelStreamRegionFiles`): meta.vxrm JSON, LRU cache,
//!   lod-directory layout, `convert_files` — 1091 lines of C++ tied to Godot
//!   `Resource`/`Mutex`/`JSON`. Lands with the `VoxelStream` trait (Phase 4).
//! - **v2→v3 legacy migration**: needs `FileAccess::insert_bytes` (grow-file-
//!   in-place); only relevant for reading old saves.
//! - **File locking** (`file_utils.h`): deferred to Phase 4 (threading).

pub mod format;
pub mod region_file;

pub use format::{RegionBlockInfo, RegionFormat, FILE_EXTENSION, FORMAT_VERSION, MAGIC};
pub use region_file::{RegionError, RegionFile};
