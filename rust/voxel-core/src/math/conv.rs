//! Explicit vector conversions (ZN-internal).
//!
//! Ported from the ZN→ZN subset of `util/math/conv.h`. The Godot⇄ZN overloads
//! live in the `voxel-gdext` binding layer (those Godot types do not exist in
//! `voxel-core`). Kept separate from the vector modules to mirror the C++
//! "avoid circular deps" rationale.

use super::vector3::{Vector3f, Vector3i};

/// `Vector3i → Vector3f` (component-wise widen). Matches `to_vec3f(Vector3i)`.
#[inline]
pub fn vec3i_to_vec3f(v: Vector3i) -> Vector3f {
    Vector3f::new(v.x as f32, v.y as f32, v.z as f32)
}

/// Floor each component and cast to `i32`. Matches `math::floor_to_int(Vector3f)`.
#[inline]
pub fn floor_to_int(v: Vector3f) -> Vector3i {
    Vector3i::new(v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32)
}

/// Round each component to nearest and cast to `i32`. Matches `math::round_to_int`.
#[inline]
pub fn round_to_int(v: Vector3f) -> Vector3i {
    Vector3i::new(v.x.round() as i32, v.y.round() as i32, v.z.round() as i32)
}

/// Ceil each component and cast to `i32`. Matches `math::ceil_to_int(Vector3f)`.
#[inline]
pub fn ceil_to_int(v: Vector3f) -> Vector3i {
    Vector3i::new(v.x.ceil() as i32, v.y.ceil() as i32, v.z.ceil() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_round_ceil_to_int() {
        let v = Vector3f::new(1.5, -1.5, 2.4);
        assert_eq!(floor_to_int(v), Vector3i::new(1, -2, 2));
        assert_eq!(round_to_int(v), Vector3i::new(2, -2, 2)); // round half-away-from-zero
        assert_eq!(ceil_to_int(v), Vector3i::new(2, -1, 3));
    }

    #[test]
    fn vec3i_to_vec3f_widen() {
        assert_eq!(
            vec3i_to_vec3f(Vector3i::new(1, 2, 3)),
            Vector3f::new(1.0, 2.0, 3.0)
        );
    }
}
