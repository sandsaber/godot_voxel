//! Stream lifetime dependency ported from `engine/streaming_dependency.h`.

use crate::streams::VoxelStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared stream handle invalidated when terrain replaces its stream setup.
pub struct StreamingDependency {
    stream: Arc<dyn VoxelStream>,
    valid: AtomicBool,
}

impl StreamingDependency {
    pub fn new(stream: Arc<dyn VoxelStream>) -> Arc<Self> {
        Arc::new(Self {
            stream,
            valid: AtomicBool::new(true),
        })
    }

    pub fn reset(slot: &mut Option<Arc<Self>>, stream: Arc<dyn VoxelStream>) -> Arc<Self> {
        if let Some(previous) = slot.take() {
            previous.invalidate();
        }
        let dependency = Self::new(stream);
        *slot = Some(dependency.clone());
        dependency
    }

    pub fn stream(&self) -> Arc<dyn VoxelStream> {
        self.stream.clone()
    }

    pub fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }

    pub fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::StreamingDependency;
    use crate::streams::{MemoryStream, VoxelStream};
    use std::sync::Arc;

    #[test]
    fn reset_invalidates_previous_dependency() {
        let first_stream: Arc<dyn VoxelStream> = Arc::new(MemoryStream::new());
        let second_stream: Arc<dyn VoxelStream> = Arc::new(MemoryStream::new());
        let mut slot = None;

        let first = StreamingDependency::reset(&mut slot, first_stream.clone());
        assert!(first.is_valid());
        assert!(Arc::ptr_eq(&first.stream(), &first_stream));

        let second = StreamingDependency::reset(&mut slot, second_stream.clone());

        assert!(!first.is_valid());
        assert!(second.is_valid());
        assert!(Arc::ptr_eq(&second.stream(), &second_stream));
        assert!(Arc::ptr_eq(slot.as_ref().unwrap(), &second));
    }
}
