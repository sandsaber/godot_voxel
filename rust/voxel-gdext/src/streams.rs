use std::sync::Arc;

use godot::prelude::*;
use voxel_core::streams::{MemoryStream, VoxelStream};

#[derive(Clone, Default)]
pub(crate) struct MemoryStreamHandle {
    stream: Arc<MemoryStream>,
}

impl MemoryStreamHandle {
    pub(crate) fn typed_stream(&self) -> Arc<MemoryStream> {
        self.stream.clone()
    }

    pub(crate) fn core_stream(&self) -> Arc<dyn VoxelStream> {
        self.stream.clone()
    }

    fn block_count(&self) -> usize {
        self.stream.len()
    }

    fn clear(&self) {
        self.stream.clear();
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamMemory {
    base: Base<Resource>,
    handle: MemoryStreamHandle,
}

#[godot_api]
impl IResource for VoxelStreamMemory {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            handle: MemoryStreamHandle::default(),
        }
    }
}

#[godot_api]
impl VoxelStreamMemory {
    #[func]
    fn get_block_count(&self) -> i32 {
        i32::try_from(self.handle.block_count()).unwrap_or(i32::MAX)
    }

    #[func]
    fn clear(&self) {
        self.handle.clear();
    }

    pub(crate) fn core_stream(&self) -> Arc<dyn VoxelStream> {
        self.handle.core_stream()
    }
}

pub(crate) fn resolve_core_stream(resource: Gd<Resource>) -> Option<Arc<dyn VoxelStream>> {
    resource
        .clone()
        .try_cast::<VoxelStreamMemory>()
        .ok()
        .map(|stream| stream.bind().core_stream())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxel_core::math::Vector3i;
    use voxel_core::storage::VoxelBuffer;

    #[test]
    fn memory_handle_exposes_one_shared_stream() {
        let handle = MemoryStreamHandle::default();
        let core = handle.typed_stream();
        core.save_block(
            Vector3i::new(2, -1, 4),
            0,
            &VoxelBuffer::with_size(Vector3i::splat(1)),
        );

        assert_eq!(handle.block_count(), 1);
        handle.clear();
        assert_eq!(core.len(), 0);
    }
}
