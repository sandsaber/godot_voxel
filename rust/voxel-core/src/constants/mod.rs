//! `constants` — engine-wide lookup tables and constants.
//!
//! Ported from `constants/`. Currently:
//! - [`cube_tables`] — `Side`/`Edge`/`Corner` enums and geometry LUTs used by
//!   the blocky and cubes meshers and any neighbor-aware voxel algorithm.
//! - [`voxel_constants`] — engine-wide terrain constants shared by storage,
//!   streams and tasks.

pub mod cube_tables;

pub mod voxel_constants {
    //! Terrain constants ported from `constants/voxel_constants.h`.

    pub const MINIMUM_LOD_DISTANCE: f32 = 16.0;
    pub const MAXIMUM_LOD_DISTANCE: f32 = 128.0;

    pub const MIN_BLOCK_SIZE: u32 = 16;
    pub const MAX_BLOCK_SIZE: u32 = 32;
    pub const MAX_BLOCK_COUNT_PER_REQUEST: usize = 4 * 4 * 4;

    pub const MAX_LOD: usize = 24;

    pub const MAX_VOLUME_EXTENT: i32 = 0x1fff_ffff;
    pub const MAX_VOLUME_SIZE: i32 = 2 * MAX_VOLUME_EXTENT;

    pub const DEFAULT_BLOCK_SIZE_PO2: u8 = 4;

    pub const DEFAULT_MIN_SUPPORTED_BLOCK_COORDINATE: i32 =
        -(MAX_VOLUME_EXTENT >> DEFAULT_BLOCK_SIZE_PO2);
    pub const DEFAULT_MAX_SUPPORTED_BLOCK_COORDINATE: i32 =
        MAX_VOLUME_EXTENT >> DEFAULT_BLOCK_SIZE_PO2;

    pub const DEFAULT_COLLISION_MARGIN: f32 = 0.04;

    pub const TASK_PRIORITY_MESH_BAND2: u8 = 10;
    pub const TASK_PRIORITY_GENERATE_BAND2: u8 = 10;
    pub const TASK_PRIORITY_LOAD_BAND2: u8 = 10;
    pub const TASK_PRIORITY_SAVE_BAND2: u8 = 9;
    pub const TASK_PRIORITY_DETAIL_TEXTURES_BAND2: u8 = 8;

    pub const TASK_PRIORITY_BAND3_DEFAULT: u8 = 10;
}
