//! Math primitives ported from `util/math/`.
//!
//! Submodules mirror the C++ header layout:
//! - [`constants`] — `Axis`, `TAU`/`PI`/`SQRT2`/`SQRT3`, `UNIT_EPSILON`.
//! - [`funcs`] — scalar math (`min`, `clamp`, `lerp`, `is_equal_approx`, …).
//! - [`vector3`] — `Vector3T<T>` and the `Vector3f` / `Vector3i` aliases.
//! - [`box3i`] — `Box3i` axis-aligned integer bounds.

pub mod box3i;
pub mod constants;
pub mod funcs;
pub mod vector3;

// Convenience re-exports at crate root path `voxel_core::math::*`.
pub use box3i::Box3i;
pub use constants::*;
pub use funcs::*;
pub use vector3::{Vector3T, Vector3d, Vector3f, Vector3i};
