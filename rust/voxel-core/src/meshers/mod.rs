//! Meshers — convert voxel data into renderable triangle meshes.
//!
//! Ported from `meshers/`. Currently includes the transvoxel (smooth SDF)
//! mesher, the cubes (blocky) mesher, the blocky (model-library) mesher
//! data layer, and the [`voxel_mesher`] trait that unifies them for the
//! terrain meshing pipeline.

pub mod blocky;
pub mod cubes;
pub mod transvoxel;
pub mod voxel_mesher;

pub use voxel_mesher::{
    CollisionSurface, MesherInput, MesherOutput, Surface, SurfaceArrays, VoxelMesher,
};
