//! Godot `RefCounted` binding for [`voxel_core::edition::VoxelToolBuffer`].
//!
//! `VoxelToolBufferGD` wraps a `VoxelBuffer` and exposes sphere/box/set_voxel
//! editing operations to GDScript.

use godot::prelude::*;
use voxel_core::edition::EditMode;
use voxel_core::math::{Vector3f, Vector3i};
use voxel_core::storage::{ChannelId, VoxelBuffer, VoxelFormat};

/// A Godot `RefCounted` that wraps a [`VoxelBuffer`] and provides voxel
/// editing operations (sphere, box, set_voxel) callable from GDScript.
#[derive(GodotClass)]
#[class(base = RefCounted, tool, rename = VoxelToolBuffer)]
pub struct VoxelToolBufferGD {
    base: Base<RefCounted>,
    buffer: VoxelBuffer,
    channel: usize,
}

#[godot_api]
impl IRefCounted for VoxelToolBufferGD {
    fn init(base: Base<RefCounted>) -> Self {
        let mut buffer = VoxelBuffer::with_size(Vector3i::splat(16));
        VoxelFormat::new().configure_buffer(&mut buffer);
        Self {
            base,
            buffer,
            channel: ChannelId::Sdf.index(),
        }
    }
}

#[godot_api]
impl VoxelToolBufferGD {
    /// Create a new VoxelToolBufferGD with a buffer of the given size.
    #[func]
    fn create_buffer(&mut self, size_x: i32, size_y: i32, size_z: i32) {
        self.buffer = VoxelBuffer::with_size(Vector3i::new(size_x, size_y, size_z));
        VoxelFormat::new().configure_buffer(&mut self.buffer);
    }

    /// Set the channel to edit (default: SDF channel). Out-of-range values
    /// are rejected instead of silently remapped; the current channel is kept.
    #[func]
    fn set_channel(&mut self, channel: i32) {
        let valid = channel >= 0 && (channel as usize) < self.buffer.channel_count();
        debug_assert!(
            valid,
            "VoxelToolBuffer.set_channel: {} out of range (channels={})",
            channel,
            self.buffer.channel_count()
        );
        if valid {
            self.channel = channel as usize;
        }
    }

    /// Run a sphere edit at world center with the given radius.
    /// mode: 0=Add, 1=Remove, 2=Set. value: voxel value for blocky mode.
    #[func]
    fn do_sphere(&mut self, cx: f64, cy: f64, cz: f64, radius: f64, mode: i32, value: i64) {
        let edit_mode = match mode {
            0 => EditMode::Add,
            1 => EditMode::Remove,
            _ => EditMode::Set,
        };
        voxel_core::edition::do_sphere(
            &mut self.buffer,
            self.channel,
            edit_mode,
            value as u64,
            Vector3f::new(cx as f32, cy as f32, cz as f32),
            radius as f32,
        );
    }

    /// Run a box edit from min to max (inclusive).
    /// mode: 0=Add, 1=Remove, 2=Set.
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn do_box(
        &mut self,
        min_x: i32,
        min_y: i32,
        min_z: i32,
        max_x: i32,
        max_y: i32,
        max_z: i32,
        mode: i32,
        value: i64,
    ) {
        let edit_mode = match mode {
            0 => EditMode::Add,
            1 => EditMode::Remove,
            _ => EditMode::Set,
        };
        voxel_core::edition::do_box(
            &mut self.buffer,
            self.channel,
            edit_mode,
            value as u64,
            Vector3i::new(min_x, min_y, min_z),
            Vector3i::new(max_x, max_y, max_z),
        );
    }

    /// Set a single voxel at the given position. Out-of-range positions are
    /// ignored (the workspace builds with `panic = "abort"`, so unchecked
    /// indexing would kill the Godot process).
    #[func]
    fn set_voxel(&mut self, x: i32, y: i32, z: i32, value: i64) {
        if !self.in_bounds(x, y, z) {
            return;
        }
        self.buffer.set_voxel(value as u64, x, y, z, self.channel);
    }

    /// Get a voxel value at the given position. Out-of-range reads return 0.
    #[func]
    fn get_voxel(&self, x: i32, y: i32, z: i32) -> i64 {
        if !self.in_bounds(x, y, z) {
            return 0;
        }
        self.buffer.get_voxel(x, y, z, self.channel) as i64
    }
}

impl VoxelToolBufferGD {
    fn in_bounds(&self, x: i32, y: i32, z: i32) -> bool {
        let size = self.buffer.size();
        let valid = x >= 0 && y >= 0 && z >= 0 && x < size.x && y < size.y && z < size.z;
        debug_assert!(
            valid,
            "VoxelToolBuffer access out of range: pos=({}, {}, {}) (size={:?})",
            x, y, z, size
        );
        valid
    }
}
