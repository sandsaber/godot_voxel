use std::path::PathBuf;
use std::sync::Arc;

use godot::prelude::*;
use voxel_core::streams::region::RegionFile;
use voxel_core::streams::{
    LoadResult, MemoryStream, VoxelLoadQuery, VoxelSaveQuery, VoxelStream, VoxelStreamError,
};

#[derive(Clone, Default)]
pub(crate) struct MemoryStreamHandle {
    stream: Arc<MemoryStream>,
}

impl MemoryStreamHandle {
    #[cfg(test)]
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
    if let Ok(stream) = resource.clone().try_cast::<VoxelStreamMemory>() {
        return Some(stream.bind().core_stream());
    }
    if let Ok(stream) = resource.clone().try_cast::<VoxelStreamRegionFiles>() {
        return Some(stream.bind().core_stream());
    }
    None
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

// ---------------------------------------------------------------------------
// VoxelStreamRegionFiles — disk persistence via .vxr region files
// ---------------------------------------------------------------------------

/// A Godot `Resource` that saves/loads voxel data to region files on disk.
/// Set the `directory` property to a writable folder, then assign this stream
/// to a [`VoxelTerrain`](crate::terrain::VoxelTerrain) to enable persistence.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamRegionFiles {
    base: Base<Resource>,
    /// Directory where `.vxr` region files are stored.
    #[var]
    directory: GString,
}

#[godot_api]
impl IResource for VoxelStreamRegionFiles {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            directory: "res://voxel_data".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelStreamRegionFiles {
    /// Build a voxel-core `Arc<dyn VoxelStream>` from this resource.
    /// Creates region files lazily in the configured directory.
    pub(crate) fn core_stream(&self) -> Arc<dyn VoxelStream> {
        let dir = self.directory.to_string();
        Arc::new(RegionFilesStream {
            directory: PathBuf::from(dir),
        })
    }
}

/// Internal stream adapter: wraps `RegionFile` operations behind the
/// `VoxelStream` trait. Each block maps to a region file via grid coords.
struct RegionFilesStream {
    directory: PathBuf,
}

const REGION_SIZE: i32 = 32;

type CoreVec3i = voxel_core::math::Vector3i;

fn region_pos(block_pos: CoreVec3i) -> (i32, i32, i32) {
    (
        block_pos.x.div_euclid(REGION_SIZE),
        block_pos.y.div_euclid(REGION_SIZE),
        block_pos.z.div_euclid(REGION_SIZE),
    )
}

#[allow(dead_code)]
fn local_pos(block_pos: CoreVec3i) -> (usize, usize, usize) {
    (
        block_pos.x.rem_euclid(REGION_SIZE) as usize,
        block_pos.y.rem_euclid(REGION_SIZE) as usize,
        block_pos.z.rem_euclid(REGION_SIZE) as usize,
    )
}

impl VoxelStream for RegionFilesStream {
    fn load_voxel_block(&self, query: VoxelLoadQuery<'_>) -> Result<LoadResult, VoxelStreamError> {
        let rp = region_pos(query.position_in_blocks);
        let local = CoreVec3i::new(
            query.position_in_blocks.x.rem_euclid(REGION_SIZE),
            query.position_in_blocks.y.rem_euclid(REGION_SIZE),
            query.position_in_blocks.z.rem_euclid(REGION_SIZE),
        );
        let filename = format!("r.{}.{}.{}.vxr", rp.0, rp.1, rp.2);
        let path = self.directory.join(&filename);
        // A missing region file is the normal "no data here" case. Any error
        // on a file that DOES exist (corrupt header, I/O failure, bad block)
        // is a real problem and must surface instead of masquerading as
        // NotFound.
        if !path.exists() {
            return Ok(LoadResult::NotFound);
        }
        let mut region = RegionFile::open(&path, false)
            .map_err(|e| VoxelStreamError::Io(format!("region open {path:?}: {e}")))?;
        match region.load_block(local, query.voxel_buffer) {
            Ok(()) => Ok(LoadResult::Found),
            Err(voxel_core::streams::region::RegionError::BlockNotFound) => {
                Ok(LoadResult::NotFound)
            }
            Err(e) => Err(VoxelStreamError::Io(format!(
                "region load {path:?} block {local:?}: {e}"
            ))),
        }
    }

    fn save_voxel_block(&self, query: VoxelSaveQuery<'_>) -> Result<(), VoxelStreamError> {
        let rp = region_pos(query.position_in_blocks);
        let local = CoreVec3i::new(
            query.position_in_blocks.x.rem_euclid(REGION_SIZE),
            query.position_in_blocks.y.rem_euclid(REGION_SIZE),
            query.position_in_blocks.z.rem_euclid(REGION_SIZE),
        );
        std::fs::create_dir_all(&self.directory).map_err(|e| {
            VoxelStreamError::Io(format!("create stream dir {:?}: {e}", self.directory))
        })?;
        let filename = format!("r.{}.{}.{}.vxr", rp.0, rp.1, rp.2);
        let path = self.directory.join(&filename);
        let mut region = RegionFile::open(&path, true)
            .map_err(|e| VoxelStreamError::Io(format!("region open {path:?}: {e:?}")))?;
        region
            .save_block(
                local,
                query.voxel_buffer,
                voxel_core::streams::compressed_data::Compression::Lz4,
            )
            .map_err(|e| VoxelStreamError::Io(format!("region save: {e:?}")))
    }
}
