//! Voxel modifiers — post-generation SDF transforms applied to voxel blocks.
//!
//! Ports the engine-agnostic half of `modifiers/voxel_modifier*.{h,cpp}`.
//! Each modifier receives a per-voxel SDF slice + world positions and blends
//! a shape SDF (sphere, mesh, etc.) into the existing values using smooth
//! union / subtract. The `ModifierStack` owns an ordered collection of
//! modifiers and applies them in sequence.

pub mod stack;

pub use stack::{
    sdf_blend, ModifierContext, ModifierStack, SdfOperation, SphereModifier, VoxelModifier,
};
