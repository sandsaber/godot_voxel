//! `meshers::cubes` — blocky/cube voxel mesher.
//!
//! Ported from `meshers/cubes/`. A binary face-culling mesher for fully-solid
//! voxels (each voxel is solid or air — no partial occupancy, unlike
//! transvoxel's SDF smoothing). Three modes exist in C++ (simple, greedy,
//! greedy+atlas); this port covers the **palette** and the **simple + greedy**
//! algorithms. The atlased mode (UV packing into an `Image`) is deferred.
//!
//! The C++ `VoxelMesher` base class (a Godot `Resource` with `Input`/`Output`
//! structs holding Godot `Array`/`Ref<Material>`/`Ref<Image>`) is intentionally
//! not ported yet — the algorithm is exposed as a free function taking raw
//! voxel bytes, matching how the transvoxel port exposes
//! `build_regular_mesh`. A shared `VoxelMesher` trait lands when a second
//! mesher needs the same shell.

pub mod arrays;
pub mod greedy;
pub mod palette;
pub mod simple;

pub use arrays::CubesArrays;
pub use palette::{ColorPalette, PALETTE_SIZE};
