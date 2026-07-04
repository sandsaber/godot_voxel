//! File-format readers for voxel assets.
//!
//! Each submodule parses a self-contained on-disk or in-memory format into
//! plain Rust data. No Godot dependency — these are pure decoders, ready for
//! reuse from either `voxel-core` tests or the `voxel-gdext` binding layer.
//!
//! ## Current
//! - [`vox`] — MagicaVoxel `.vox` scene format (models, scene graph, palette).
//!
//! ## Planned (Phase 3+)
//! - `region` — godot_voxel's region-file archive format
//!   (`streams/region/region_file.cpp`).
//! - `block_serializer` — `streams/voxel_block_serializer.cpp`.

pub mod vox;
