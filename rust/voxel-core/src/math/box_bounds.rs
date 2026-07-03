//! Alternative integer boxes storing explicit min/max corners.
//!
//! Ported from `util/math/box_bounds_2i.h` and `util/math/box_bounds_3i.h`.
//! Unlike [`super::box3i::Box3i`] / [`super::box2i::Box2i`] (which store
//! position+size), these store `min_pos` and an **exclusive** `max_pos`. That
//! makes intersection / containment checks slightly cheaper (no `position+size`
//! additions) — they're used in hot spatial-query paths.

use super::vector2::Vector2i;
use super::vector3::Vector3i;

/// 2D integer box stored as min + exclusive-max. Matches `BoxBounds2i`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct BoxBounds2i {
    pub min_pos: Vector2i,
    pub max_pos: Vector2i, // Exclusive
}

impl BoxBounds2i {
    pub const fn new(min: Vector2i, max: Vector2i) -> Self {
        Self {
            min_pos: min,
            max_pos: max,
        }
    }

    /// Convert from a position+size box. Matches `BoxBounds2i(Box2i)`.
    pub fn from_box(pos: Vector2i, size: Vector2i) -> Self {
        Self::new(pos, pos + size)
    }

    pub fn from_position_size(pos: Vector2i, size: Vector2i) -> Self {
        Self::new(pos, pos + size)
    }

    /// Inclusive min/max on both ends (max is converted to exclusive internally).
    /// Matches `from_min_max_included`.
    pub fn from_min_max_included(minp: Vector2i, maxp: Vector2i) -> Self {
        Self::new(minp, maxp + Vector2i::splat(1))
    }

    /// Single-cell box covering just `pos`. Matches `from_position`.
    pub fn from_position(pos: Vector2i) -> Self {
        Self::new(pos, pos + Vector2i::splat(1))
    }

    /// Unbounded box covering the whole `i32` range. Matches `from_everywhere`.
    pub fn from_everywhere() -> Self {
        Self::new(Vector2i::splat(i32::MIN), Vector2i::splat(i32::MAX))
    }

    #[inline]
    pub fn intersects(&self, other: &BoxBounds2i) -> bool {
        !(self.max_pos.x < other.min_pos.x
            || self.max_pos.y < other.min_pos.y
            || self.min_pos.x > other.max_pos.x
            || self.min_pos.y > other.max_pos.y)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.min_pos.x >= self.max_pos.x || self.min_pos.y >= self.max_pos.y
    }

    #[inline]
    pub fn size(&self) -> Vector2i {
        self.max_pos - self.min_pos
    }
}

/// 3D integer box stored as min + exclusive-max. Matches `BoxBounds3i`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct BoxBounds3i {
    pub min_pos: Vector3i,
    pub max_pos: Vector3i, // Exclusive
}

impl BoxBounds3i {
    pub const fn new(min: Vector3i, max: Vector3i) -> Self {
        Self {
            min_pos: min,
            max_pos: max,
        }
    }

    /// Convert from a position+size box. Matches `BoxBounds3i(Box3i)`.
    pub fn from_box(pos: Vector3i, size: Vector3i) -> Self {
        Self::new(pos, pos + size)
    }

    pub fn from_position_size(pos: Vector3i, size: Vector3i) -> Self {
        Self::new(pos, pos + size)
    }

    /// Inclusive min/max on both ends. Matches `from_min_max_included`.
    pub fn from_min_max_included(minp: Vector3i, maxp: Vector3i) -> Self {
        Self::new(minp, maxp + Vector3i::splat(1))
    }

    /// Single-cell box covering just `pos`. Matches `from_position`.
    pub fn from_position(pos: Vector3i) -> Self {
        Self::new(pos, pos + Vector3i::splat(1))
    }

    /// Unbounded box covering the whole `i32` range. Matches `from_everywhere`.
    pub fn from_everywhere() -> Self {
        Self::new(Vector3i::splat(i32::MIN), Vector3i::splat(i32::MAX))
    }

    #[inline]
    pub fn intersects(&self, other: &BoxBounds3i) -> bool {
        !(self.max_pos.x < other.min_pos.x
            || self.max_pos.y < other.min_pos.y
            || self.max_pos.z < other.min_pos.z
            || self.min_pos.x > other.max_pos.x
            || self.min_pos.y > other.max_pos.y
            || self.min_pos.z > other.max_pos.z)
    }

    /// Half-open containment: `min_pos <= p < max_pos`. Matches `contains`.
    #[inline]
    pub fn contains(&self, p: Vector3i) -> bool {
        p.x >= self.min_pos.x
            && p.y >= self.min_pos.y
            && p.z >= self.min_pos.z
            && p.x < self.max_pos.x
            && p.y < self.max_pos.y
            && p.z < self.max_pos.z
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.min_pos.x >= self.max_pos.x
            || self.min_pos.y >= self.max_pos.y
            || self.min_pos.z >= self.max_pos.z
    }

    #[inline]
    pub fn size(&self) -> Vector3i {
        self.max_pos - self.min_pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds2i_constructors() {
        let b = BoxBounds2i::from_position_size(Vector2i::new(1, 2), Vector2i::new(3, 4));
        assert_eq!(b.min_pos, Vector2i::new(1, 2));
        assert_eq!(b.max_pos, Vector2i::new(4, 6));

        let inc = BoxBounds2i::from_min_max_included(Vector2i::new(0, 0), Vector2i::new(3, 3));
        assert_eq!(inc.max_pos, Vector2i::new(4, 4));

        let one = BoxBounds2i::from_position(Vector2i::new(5, 5));
        assert_eq!(one.size(), Vector2i::splat(1));

        let all = BoxBounds2i::from_everywhere();
        assert!(!all.is_empty());
    }

    #[test]
    fn bounds2i_intersects_and_empty() {
        let a = BoxBounds2i::new(Vector2i::new(0, 0), Vector2i::new(4, 4));
        // Overlapping region.
        assert!(a.intersects(&BoxBounds2i::new(Vector2i::new(2, 2), Vector2i::new(6, 6))));
        // C++ uses strict `<`/`>`, so boxes that merely touch edges/corners still
        // report as intersecting (conservative AABB test). (0,0)-(4,4) vs (4,4)-(6,6)
        // shares the corner cell (4,4) under this rule.
        assert!(a.intersects(&BoxBounds2i::new(Vector2i::new(4, 4), Vector2i::new(6, 6))));
        // Truly disjoint.
        assert!(!a.intersects(&BoxBounds2i::new(Vector2i::new(5, 5), Vector2i::new(6, 6))));
        assert!(BoxBounds2i::new(Vector2i::new(0, 0), Vector2i::new(0, 4)).is_empty());
    }

    #[test]
    fn bounds3i_contains_half_open() {
        let b = BoxBounds3i::new(Vector3i::zero(), Vector3i::new(4, 4, 4));
        assert!(b.contains(Vector3i::new(0, 0, 0)));
        assert!(b.contains(Vector3i::new(3, 3, 3)));
        assert!(!b.contains(Vector3i::new(4, 0, 0))); // exclusive max
        assert!(!b.contains(Vector3i::new(-1, 0, 0)));
    }

    #[test]
    fn bounds3i_intersects_and_size() {
        let a = BoxBounds3i::new(Vector3i::zero(), Vector3i::new(4, 4, 4));
        // Overlapping region.
        assert!(a.intersects(&BoxBounds3i::new(
            Vector3i::new(3, 3, 3),
            Vector3i::new(9, 9, 9)
        )));
        // C++ uses strict `<`/`>`, so touching corner still counts as intersecting.
        assert!(a.intersects(&BoxBounds3i::new(
            Vector3i::new(4, 4, 4),
            Vector3i::new(9, 9, 9)
        )));
        // Truly disjoint (gap of at least one cell on every axis).
        assert!(!a.intersects(&BoxBounds3i::new(
            Vector3i::new(5, 5, 5),
            Vector3i::new(9, 9, 9)
        )));
        assert_eq!(a.size(), Vector3i::splat(4));
    }

    #[test]
    fn from_box_matches_box3i_semantics() {
        // position(1,1,1) + size(2,2,2) => max_pos exclusive (3,3,3).
        let b = BoxBounds3i::from_box(Vector3i::new(1, 1, 1), Vector3i::new(2, 2, 2));
        assert_eq!(b.min_pos, Vector3i::new(1, 1, 1));
        assert_eq!(b.max_pos, Vector3i::new(3, 3, 3));
        assert!(b.contains(Vector3i::new(2, 2, 2)));
        assert!(!b.contains(Vector3i::new(3, 3, 3)));
    }
}
