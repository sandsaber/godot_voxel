//! `voxel-core` — engine-agnostic voxel engine core.
//!
//! Rust port of the engine-independent parts of [godot_voxel](https://github.com/Zylann/godot_voxel).
//! This crate deliberately has **no** dependency on Godot: all math, storage,
//! meshing and generation logic lives here and is unit-testable without an
//! engine. The thin Godot binding lives in the separate `voxel-gdext` crate.
//!
//! ## Status
//! This crate is under active development as part of the Rust migration pilot
//! (Phase 4 started; Phases 1-3 audited). See `MIGRATION_PLAN.md` at the
//! repository root for context.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod constants;
pub mod containers;
pub mod edition;
pub mod engine;
pub mod format;
pub mod generators;
pub mod hash;
pub mod io;
pub mod math;
pub mod memory;
pub mod meshers;
pub mod storage;
pub mod streams;
pub mod string;
pub mod tasks;
pub mod terrain;
pub mod testing;
pub mod thread;

/// Crate version (matches `Cargo.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
