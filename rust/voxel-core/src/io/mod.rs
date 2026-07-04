//! I/O utilities.
//!
//! Ported from `util/io/`. Submodules:
//! - [`serialization`] — endianness-aware byte readers/writers.
//! - [`text_writer`] — buffered character output stream.
//! - [`log`] — verbose flag + print/error/warning helpers.
//! - [`voxel_file`] — file-I/O trait ([`voxel_file::VoxelFile`]) standing in
//!   for Godot `FileAccess`, used by the region-file format.
//!
//! ## Deferred
//!
//! **Deferred to Phase 4** (terrain/threading): `file_locker.h` — per-path
//! read/write locking built on the newly ported thread primitives.

pub mod log;
pub mod serialization;
pub mod text_writer;
pub mod voxel_file;
