//! Voxel storage primitives ported from `storage/`.
//!
//! This is the minimal subset needed by the Phase 0 transvoxel pilot.
//! Full `VoxelBuffer` (with compression, multiple allocators, format round-tripping)
//! is migrated in Phase 3.

pub mod buffer;
pub mod depth;

pub use buffer::{DenseVoxelBuffer, VoxelBufferRead};
pub use depth::ChannelDepth;
