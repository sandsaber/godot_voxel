//! 32-bit float quaternion.
//!
//! Ported from `util/math/quaternionf.h`. A minimal `(x, y, z, w)` quaternion
//! with length/normalize helpers — the full rotation API (slerp, from euler,
//! rotate vector) arrives with `transform3f` / when the terrain layer needs it.

use super::funcs;

/// 32-bit float quaternion `(x, y, z, w)`. Matches C++ `Quaternionf`. Default is
/// the identity `(0, 0, 0, 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct Quaternionf {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quaternionf {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Identity rotation `(0, 0, 0, 1)`.
    pub const IDENTITY: Self = Self::new(0.0, 0.0, 0.0, 1.0);
}

impl Default for Quaternionf {
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl core::ops::Div<f32> for Quaternionf {
    type Output = Self;
    #[inline]
    fn div(self, d: f32) -> Self {
        Self::new(self.x / d, self.y / d, self.z / d, self.w / d)
    }
}

/// Free math functions on `Quaternionf`, mirroring `zylann::math::` overloads.
pub mod math {
    use super::{funcs, Quaternionf};

    #[inline]
    pub fn length_squared(q: Quaternionf) -> f32 {
        q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w
    }

    #[inline]
    pub fn length(q: Quaternionf) -> f32 {
        funcs::sqrt_f32(length_squared(q))
    }

    #[inline]
    pub fn normalized(q: Quaternionf) -> Quaternionf {
        q / length(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_identity() {
        assert_eq!(Quaternionf::default(), Quaternionf::IDENTITY);
        assert_eq!(Quaternionf::default(), Quaternionf::new(0.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn length_and_normalize() {
        let q = Quaternionf::new(0.0, 0.0, 0.0, 2.0);
        assert_eq!(math::length_squared(q), 4.0);
        assert!((math::length(q) - 2.0).abs() < 1e-5);
        let n = math::normalized(q);
        assert!((math::length(n) - 1.0).abs() < 1e-5);
    }
}
