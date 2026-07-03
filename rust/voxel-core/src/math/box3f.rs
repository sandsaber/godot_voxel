//! Axis-aligned 3D box with floating-point coordinates.
//!
//! Ported from `util/math/box3f.h`. Unlike [`super::box3i::Box3i`] (which uses
//! position+size), this stores explicit `min`/`max` corners — matching the C++
//! `Box3fT<T>` template. Both corners are **inclusive**.

use super::vector3::{Vector3T, Vector3f};

/// Axis-aligned 3D box with float min/max corners (inclusive). Matches `Box3fT<T>`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Box3fT<T: Copy> {
    pub min: Vector3T<T>,
    pub max: Vector3T<T>,
}

impl<T: Copy + PartialOrd> Box3fT<T> {
    pub fn from_min_max(min: Vector3T<T>, max: Vector3T<T>) -> Self {
        Self { min, max }
    }
}

impl Box3fT<f32> {
    pub fn from_min_size(min: Vector3f, size: Vector3f) -> Self {
        Self {
            min,
            max: min + size,
        }
    }

    pub fn from_center_half_size(center: Vector3f, hs: Vector3f) -> Self {
        Self {
            min: center - hs,
            max: center + hs,
        }
    }

    /// Inclusive containment test. Matches `contains`.
    #[inline]
    pub fn contains(&self, p: Vector3f) -> bool {
        p.x >= self.min.x
            && p.y >= self.min.y
            && p.z >= self.min.z
            && p.x <= self.max.x
            && p.y <= self.max.y
            && p.z <= self.max.z
    }

    /// Squared distance from the box to `p` (0 if inside). Matches `distance_squared`:
    /// `d = max(min - p, p - max, 0)` componentwise, then `length_squared`.
    #[inline]
    pub fn distance_squared(&self, p: Vector3f) -> f32 {
        let dx = (self.min.x - p.x).max(p.x - self.max.x).max(0.0);
        let dy = (self.min.y - p.y).max(p.y - self.max.y).max(0.0);
        let dz = (self.min.z - p.z).max(p.z - self.max.z).max(0.0);
        dx * dx + dy * dy + dz * dz
    }
}

/// 32-bit float alias. Matches C++ `Box3f = Box3fT<float>`.
pub type Box3f = Box3fT<f32>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctors_and_contains() {
        let b = Box3f::from_min_max(
            Vector3f::new(-1.0, -1.0, -1.0),
            Vector3f::new(1.0, 1.0, 1.0),
        );
        assert!(b.contains(Vector3f::zero()));
        assert!(b.contains(Vector3f::new(1.0, 1.0, 1.0))); // inclusive
        assert!(!b.contains(Vector3f::new(1.5, 0.0, 0.0)));

        let c = Box3f::from_center_half_size(Vector3f::zero(), Vector3f::splat(2.0));
        assert_eq!(c.min, Vector3f::new(-2.0, -2.0, -2.0));
        assert_eq!(c.max, Vector3f::new(2.0, 2.0, 2.0));

        let s = Box3f::from_min_size(Vector3f::zero(), Vector3f::new(4.0, 4.0, 4.0));
        assert_eq!(s.max, Vector3f::new(4.0, 4.0, 4.0));
    }

    #[test]
    fn distance_squared_inside_is_zero() {
        let b = Box3f::from_min_max(Vector3f::zero(), Vector3f::new(2.0, 2.0, 2.0));
        assert_eq!(b.distance_squared(Vector3f::new(1.0, 1.0, 1.0)), 0.0);
        // Just outside along one axis -> 1.0.
        assert!((b.distance_squared(Vector3f::new(3.0, 1.0, 1.0)) - 1.0).abs() < 1e-5);
        // Outside along two axes -> 1^2 + 1^2 = 2.
        assert!((b.distance_squared(Vector3f::new(3.0, 3.0, 1.0)) - 2.0).abs() < 1e-5);
    }
}
