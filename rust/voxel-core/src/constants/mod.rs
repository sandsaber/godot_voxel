//! `constants` — engine-wide lookup tables and constants.
//!
//! Ported from `constants/`. Currently:
//! - [`cube_tables`] — `Side`/`Edge`/`Corner` enums and geometry LUTs used by
//!   the blocky and cubes meshers and any neighbor-aware voxel algorithm.

pub mod cube_tables;
