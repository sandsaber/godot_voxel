//! `meshers::blocky` — Minecraft-style blocky voxel mesher.
//!
//! Ported from `meshers/blocky/`. Produces meshes from a voxel-type channel
//! using a [`baked_library::BakedLibrary`] of pre-baked models. Unlike the
//! cubes mesher (which synthesizes one quad per face boundary), the blocky
//! mesher looks up each voxel's model in the library and emits its baked
//! geometry, applying neighbor-based face culling and ambient occlusion.
//!
//! ## Current
//! - [`baked_library`] — `BakedModel`/`BakedLibrary`/`BakedFluid` plain-data
//!   structs (no Godot dependency). The model-baking algorithm that populates
//!   these lands next.
//!
//! ## Planned (Phase 3)
//! - `bake` — side-culling matrix generation + cutout-surface baking.
//! - `mesher` — `generate_mesh<T>` core algorithm (face culling + AO).
//! - `lod_skirts` — LOD seam-skirt appending.
//! - `shadow_occluders` — shadow geometry generation.
//!
//! ## Deferred (Phase 5)
//! - The Godot `Resource` / `Ref<Material>` / editor layer (`VoxelMesherBlocky`,
//!   `VoxelBlockyLibraryBase`, `VoxelBlockyModel*`).

pub mod baked_library;

pub use baked_library::{
    Aabb, BakedFluid, BakedLibrary, BakedModel, BakedModelMesh, DynamicBitset, FluidSurface,
    ModelSurface, SideSurface, AIR_ID, FLUID_BOTTOM_HEIGHT, FLUID_TOP_HEIGHT, MAX_FLUIDS,
    MAX_MATERIALS, MAX_MODELS, MAX_SURFACES, NULL_FLUID_INDEX,
};
