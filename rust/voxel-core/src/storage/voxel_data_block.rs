//! Sparse storage block for one chunk of voxel data.

use crate::storage::VoxelBuffer;

#[derive(Debug)]
pub struct VoxelDataBlock {
    voxels: Option<VoxelBuffer>,
    lod_index: u8,
    needs_lodding: bool,
    modified: bool,
    edited: bool,
}

impl VoxelDataBlock {
    pub const fn empty(lod_index: u8) -> Self {
        Self {
            voxels: None,
            lod_index,
            needs_lodding: false,
            modified: false,
            edited: false,
        }
    }

    pub const fn with_voxels(voxels: VoxelBuffer, lod_index: u8) -> Self {
        Self {
            voxels: Some(voxels),
            lod_index,
            needs_lodding: false,
            modified: false,
            edited: false,
        }
    }

    pub const fn lod_index(&self) -> u8 {
        self.lod_index
    }

    pub const fn has_voxels(&self) -> bool {
        self.voxels.is_some()
    }

    pub fn voxels(&self) -> &VoxelBuffer {
        self.voxels
            .as_ref()
            .expect("voxel data block has no voxels")
    }

    pub fn voxels_mut(&mut self) -> &mut VoxelBuffer {
        self.voxels
            .as_mut()
            .expect("voxel data block has no voxels")
    }

    pub fn set_voxels(&mut self, voxels: VoxelBuffer) {
        self.voxels = Some(voxels);
    }

    pub fn clear_voxels(&mut self) {
        self.voxels = None;
        self.edited = false;
    }

    pub const fn is_modified(&self) -> bool {
        self.modified
    }

    pub const fn set_modified(&mut self, modified: bool) {
        self.modified = modified;
    }

    pub const fn needs_lodding(&self) -> bool {
        self.needs_lodding
    }

    pub const fn set_needs_lodding(&mut self, needs_lodding: bool) {
        self.needs_lodding = needs_lodding;
    }

    pub const fn is_edited(&self) -> bool {
        self.edited
    }

    pub const fn set_edited(&mut self, edited: bool) {
        self.edited = edited;
    }
}

#[cfg(test)]
mod tests {
    use super::VoxelDataBlock;
    use crate::math::Vector3i;
    use crate::storage::{ChannelId, VoxelBuffer};

    #[test]
    fn empty_block_tracks_lod_and_has_no_voxels() {
        let block = VoxelDataBlock::empty(3);

        assert_eq!(block.lod_index(), 3);
        assert!(!block.has_voxels());
        assert!(!block.is_modified());
        assert!(!block.is_edited());
        assert!(!block.needs_lodding());
    }

    #[test]
    fn block_with_voxels_exposes_flags_and_buffer() {
        let mut voxels = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        voxels.set_voxel(7, 1, 0, 0, ChannelId::Type.index());
        let mut block = VoxelDataBlock::with_voxels(voxels, 1);

        assert!(block.has_voxels());
        assert_eq!(
            block.voxels().get_voxel(1, 0, 0, ChannelId::Type.index()),
            7
        );

        block.set_modified(true);
        block.set_edited(true);
        block.set_needs_lodding(true);

        assert!(block.is_modified());
        assert!(block.is_edited());
        assert!(block.needs_lodding());

        block.clear_voxels();
        assert!(!block.has_voxels());
        assert!(!block.is_edited());
        assert!(block.is_modified());
        assert!(block.needs_lodding());
    }
}
