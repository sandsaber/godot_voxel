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
//! read/write locking; depends on `util/thread/{mutex,rw_lock}.h` which is not
//! yet ported. Will be revisited when the thread primitives land.

pub mod log;
pub mod serialization;
pub mod text_writer;
pub mod voxel_file;
