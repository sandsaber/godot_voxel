//! 3D affine transform (32-bit float): a [`Basis3f`] plus an origin.
//!
//! Ported from `util/math/transform3f.h`.

use super::basis3f::Basis3f;
use super::vector3::Vector3f;

/// 3D transform = rotation/scale basis + translation. Matches C++ `Transform3f`.
/// Default is identity (basis identity, origin zero).
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct Transform3f {
    pub basis: Basis3f,
    pub origin: Vector3f,
}

impl Default for Transform3f {
    #[inline]
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform3f {
    pub const fn identity() -> Self {
        Self {
            basis: Basis3f::identity(),
            origin: Vector3f::new(0.0, 0.0, 0.0),
        }
    }

    pub const fn new(basis: Basis3f, origin: Vector3f) -> Self {
        Self { basis, origin }
    }

    /// Transform a point: `basis.xform(v) + origin`. Matches `xform`.
    #[inline]
    pub fn xform(&self, v: Vector3f) -> Vector3f {
        self.basis.xform(v) + self.origin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_xform_is_identity() {
        let t = Transform3f::identity();
        let v = Vector3f::new(1.0, 2.0, 3.0);
        assert_eq!(t.xform(v), v);
    }

    #[test]
    fn translation_is_applied() {
        let t = Transform3f::new(Basis3f::identity(), Vector3f::new(10.0, 0.0, 0.0));
        assert_eq!(t.xform(Vector3f::zero()), Vector3f::new(10.0, 0.0, 0.0));
    }
}
