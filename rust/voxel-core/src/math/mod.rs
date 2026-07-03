//! Math primitives ported from `util/math/`.
//!
//! Submodules mirror the C++ header layout:
//! - [`constants`] — `Axis`, `TAU`/`PI`/`SQRT2`/`SQRT3`, `UNIT_EPSILON`.
//! - [`funcs`] — scalar math (`min`, `clamp`, `lerp`, `is_equal_approx`, …).
//! - [`vector3`] — `Vector3T<T>` and the `Vector3f` / `Vector3i` aliases.

pub mod constants;
pub mod funcs;
pub mod vector3;

// Convenience re-exports at crate root path `voxel_core::math::*`.
pub use constants::*;
pub use funcs::*;
pub use vector3::{
    Vector3d, Vector3f, Vector3i, Vector3T,
};
