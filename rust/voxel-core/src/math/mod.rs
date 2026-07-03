//! Math primitives ported from `util/math/`.
//!
//! Submodules mirror the C++ header layout:
//! - [`constants`] — `Axis`, `TAU`/`PI`/`SQRT2`/`SQRT3`, `UNIT_EPSILON`.
//! - [`funcs`] — scalar math (`min`, `clamp`, `lerp`, `is_equal_approx`, …).
//! - [`vector3`] — `Vector3T<T>` and the `Vector3f` / `Vector3i` aliases.
//! - [`vector2`] — `Vector2T<T>` and the `Vector2f` / `Vector2i` aliases.
//! - [`box3i`] — `Box3i` axis-aligned integer bounds.
//! - [`box2i`] — `Box2i` axis-aligned integer bounds.
//! - [`sdf`] — scalar signed-distance-field primitives.

pub mod basis3f;
pub mod box2f;
pub mod box2i;
pub mod box3f;
pub mod box3i;
pub mod box_bounds;
pub mod color;
pub mod constants;
pub mod conv;
pub mod funcs;
pub mod interval;
pub mod ortho_basis;
pub mod quaternion;
pub mod sdf;
pub mod transform3f;
pub mod triangle;
pub mod vector2;
pub mod vector3;
pub mod vector3i16;
pub mod vector4;

// Convenience re-exports at crate root path `voxel_core::math::*`.
pub use basis3f::Basis3f;
pub use box2f::Box2f;
pub use box2i::Box2i;
pub use box3f::{Box3f, Box3fT};
pub use box3i::Box3i;
pub use box_bounds::{BoxBounds2i, BoxBounds3i};
pub use color::{Color, Color8};
pub use constants::*;
pub use funcs::*;
pub use interval::{Interval, Interval2, Interval3};
pub use ortho_basis::{OrthoBasis, OrthoRotationId};
pub use quaternion::Quaternionf;
pub use transform3f::Transform3f;
pub use vector2::{Vector2T, Vector2d, Vector2f, Vector2i};
pub use vector3::{Vector3T, Vector3d, Vector3f, Vector3i};
pub use vector3i16::Vector3i16;
pub use vector4::{Vector4T, Vector4d, Vector4f, Vector4i};
