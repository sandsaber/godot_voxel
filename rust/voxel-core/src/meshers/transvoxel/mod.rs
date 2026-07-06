//! Transvoxel mesher — Marching Cubes with LOD transitions.
//!
//! Ported from `meshers/transvoxel/`. Phase 0 implements the regular-cell
//! path (`build_regular_mesh`) used to extract a smooth surface from an SDF
//! voxel volume. Transition meshes (between LOD levels) and the mixel4
//! texture-blend material mode are migrated in a later sub-phase.
//!
//! ## References
//! - Eric Lengyel, "Voxel-Based Terrain for Real-Time Virtual Simulations"
//!   <http://transvoxel.org/>
//! - C++ source: `meshers/transvoxel/transvoxel.cpp`

pub mod regular;
pub mod regular_tables;
pub mod structures;

pub use regular::{
    build_regular_mesh, BuildRegularMeshParams, RegularMesherInput, MAX_PADDING, MIN_PADDING,
};
pub use structures::{Cache, MeshArrays, ReuseCell};
