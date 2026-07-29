//! `generators` — procedural voxel terrain generation.
//!
//! Each generator fills a [`VoxelBuffer`] block with voxel data, typically from
//! a heightmap or noise function. Ported from `generators/`. The Godot
//! `VoxelGenerator` `Resource` (with its RWLock, GPU hooks, caching and async
//! task machinery) is split in two here: a thin synchronous [`VoxelGenerator`]
//! trait (the part generators themselves implement) lives in [`base`], and the
//! engine/threading layer (streaming, LOD integration and terrain ownership) is
//! layered in during Phase 4.
//!
//! ## Current
//! - [`base`] — the [`VoxelGenerator`] trait, [`VoxelQueryData`], [`GenResult`],
//!   and the shared [`base::generate_heightmap`] helper.
//! - [`simple`] — `Waves` and `Flat` generators (math-pure, no noise deps).
//!
//! ## Planned (Phase 3+)
//! - `simple::Noise` — 3D SDF via `fastnoise-lite` (pure Rust).
//! - `simple::HeightmapNoise` — 2D noise + curve heightmap.
//! - `graph` — runtime graph generator (depends on `string::expression_parser`,
//!   deferred from Phase 1).

pub mod base;
pub mod fast_noise2;
pub mod graph;
pub mod simple;
