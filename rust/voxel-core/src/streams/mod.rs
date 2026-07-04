//! `streams` — serialization formats for voxel block persistence.
//!
//! Each submodule handles one of godot_voxel's internal on-disk / in-memory
//! block representations. These are the formats the terrain layer round-trips
//! through its block cache; they are **not** public asset formats like the
//! `.vox` parser in [`crate::format::vox`].
//!
//! ## Current
//! - [`instance_data`] — lossy-compressed per-block instance transforms
//!   (instanced grass / detail).
//!
//! ## Planned (Phase 3+)
//! - `compressed_data` — LZ4/ZSTD compression envelope (needs the `lz4` crate).
//! - `block_serializer` — `VoxelBuffer` ↔ bytes, depends on `compressed_data`.

pub mod instance_data;
