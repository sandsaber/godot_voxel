//! Voxel storage primitives ported from `storage/`.
//!
//! This is the minimal subset needed by the Phase 0 transvoxel pilot.
//! Full `VoxelBuffer` (with compression, multiple allocators, format round-tripping)
//! is migrated in Phase 3.

pub mod buffer;
pub mod depth;
pub mod funcs;
pub mod voxel_buffer;
pub mod voxel_format;
pub mod voxel_memory_pool;

pub use buffer::{DenseVoxelBuffer, VoxelBufferRead};
pub use depth::ChannelDepth;
pub use voxel_buffer::{Allocator, Channel, ChannelId, Compression, VoxelBuffer};
pub use voxel_format::{DepthRange, VoxelFormat};
pub use voxel_memory_pool::VoxelMemoryPool;
