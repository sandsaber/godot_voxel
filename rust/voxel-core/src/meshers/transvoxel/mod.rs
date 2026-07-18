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
pub mod transition;
pub mod transition_tables;

pub use regular::{
    build_regular_mesh, BuildRegularMeshParams, RegularMesherInput, MAX_PADDING, MIN_PADDING,
};
pub use structures::{Cache, MeshArrays, ReuseCell, ReuseTransitionCell};
pub use transition::{
    build_transition_mesh, SIDE_COUNT, SIDE_NEGATIVE_X, SIDE_NEGATIVE_Y, SIDE_NEGATIVE_Z,
    SIDE_POSITIVE_X, SIDE_POSITIVE_Y, SIDE_POSITIVE_Z,
};
pub use transition_tables::{
    get_transition_cell_class, get_transition_cell_data, TransitionCellData,
};
