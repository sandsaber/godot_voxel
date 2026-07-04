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

pub mod block_serializer;
pub mod compressed_data;
pub mod instance_data;
