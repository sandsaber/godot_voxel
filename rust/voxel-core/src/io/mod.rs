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
//! Phase 4 adds [`file_locker`] for per-path read/write coordination used by
//! stream backends.

pub mod file_locker;
pub mod log;
pub mod serialization;
pub mod text_writer;
pub mod voxel_file;

pub use file_locker::FileLocker;
