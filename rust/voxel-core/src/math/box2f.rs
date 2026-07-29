//! Axis-aligned 2D box with floating-point coordinates.
//!
//! Ported from `util/math/box2f.h`. Stores explicit `min`/`max` corners
//! (matching the C++ class). The 2D analogue of [`super::box3f::Box3f`].
//!
//! The C++ `difference()` takes a callback and a `SmallVector`-backed
//! `difference_to_vec`; here `difference` returns a `Vec<Box2f>` directly (up to
//! 4 slabs), mirroring how [`super::box3i::Box3i::difference`] was ported.

use super::funcs;
use super::vector2::Vector2f;

/// Axis-aligned 2D box with float min/max corners. Matches `Box2f`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Box2f {
    pub min: Vector2f,
    pub max: Vector2f,
}

impl Box2f {
    pub const fn from_min_max(min: Vector2f, max: Vector2f) -> Self {
        Self { min, max }
    }

    pub fn from_min_size(min: Vector2f, size: Vector2f) -> Self {
        Self {
            min,
            max: min + size,
        }
    }

    /// Overlap test (open on the far edge, matching the C++ `>=` comparisons).
    /// Two boxes that merely touch do **not** intersect.
    #[inline]
    pub fn intersects(&self, other: &Box2f) -> bool {
        if self.min.x >= other.max.x {
            return false;
        }
        if self.min.y >= other.max.y {
            return false;
        }
        if other.min.x >= self.max.x {
            return false;
        }
        if other.min.y >= self.max.y {
            return false;
        }
        true
    }

    /// Clamp both corners into `lim`. Matches `clip`.
    pub fn clip(&mut self, lim: Box2f) {
        self.min.x = funcs::clamp(self.min.x, lim.min.x, lim.max.x);
        self.min.y = funcs::clamp(self.min.y, lim.min.y, lim.max.y);
        self.max.x = funcs::clamp(self.max.x, lim.min.x, lim.max.x);
        self.max.y = funcs::clamp(self.max.y, lim.min.y, lim.max.y);
    }

    /// Subtract `b` from `self`, returning the (up to 4) boxes covering the
    /// remaining area. If `b` does not intersect `self`, returns `[self]`.
    /// Matches `difference` / `difference_to_vec`.
    ///
    /// ```text
    /// o-----------o                 o-----o-----o
    /// | A         |                 | C1  | C2  |
    /// |     o-----+---o             |     o-----o
    /// |     |     |   |   A - B =>  |     |
    /// o-----+-----o   |             o-----o
    ///       | B       |
    ///       o---------o
    /// ```
    pub fn difference(&self, b: Box2f) -> Vec<Box2f> {
        let mut out = Vec::new();
        if !self.intersects(&b) {
            out.push(*self);
            return out;
        }

        let mut a = *self;

        if a.min.x < b.min.x {
            out.push(Box2f::from_min_max(a.min, Vector2f::new(b.min.x, a.max.y)));
            a.min.x = b.min.x;
        }
        if a.min.y < b.min.y {
            out.push(Box2f::from_min_max(a.min, Vector2f::new(a.max.x, b.min.y)));
            a.min.y = b.min.y;
        }

        if a.max.x > b.max.x {
            out.push(Box2f::from_min_max(Vector2f::new(b.max.x, a.min.y), a.max));
            a.max.x = b.max.x;
        }
        if a.max.y > b.max.y {
            out.push(Box2f::from_min_max(Vector2f::new(a.min.x, b.max.y), a.max));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctors() {
        let s = Box2f::from_min_size(Vector2f::new(1.0, 2.0), Vector2f::new(3.0, 4.0));
        assert_eq!(s.min, Vector2f::new(1.0, 2.0));
        assert_eq!(s.max, Vector2f::new(4.0, 6.0));

        let m = Box2f::from_min_max(Vector2f::new(0.0, 0.0), Vector2f::new(5.0, 5.0));
        assert_eq!(m.max, Vector2f::new(5.0, 5.0));
    }

    #[test]
    fn intersects_open_far_edge() {
        let a = Box2f::from_min_max(Vector2f::new(0.0, 0.0), Vector2f::new(4.0, 4.0));
        // Overlapping.
        assert!(a.intersects(&Box2f::from_min_max(
            Vector2f::new(2.0, 2.0),
            Vector2f::new(6.0, 6.0)
        )));
        // Touching on the far edge is NOT an intersection.
        assert!(!a.intersects(&Box2f::from_min_max(
            Vector2f::new(4.0, 0.0),
            Vector2f::new(6.0, 4.0)
        )));
        // Disjoint.
        assert!(!a.intersects(&Box2f::from_min_max(
            Vector2f::new(10.0, 10.0),
            Vector2f::new(12.0, 12.0)
        )));
    }

    #[test]
    fn clip_clamps_corners() {
        let mut b = Box2f::from_min_max(Vector2f::new(-2.0, -2.0), Vector2f::new(10.0, 10.0));
        b.clip(Box2f::from_min_max(
            Vector2f::new(0.0, 0.0),
            Vector2f::new(4.0, 4.0),
        ));
        assert_eq!(b.min, Vector2f::new(0.0, 0.0));
        assert_eq!(b.max, Vector2f::new(4.0, 4.0));
    }

    #[test]
    fn difference_disjoint_returns_self() {
        let a = Box2f::from_min_max(Vector2f::new(0.0, 0.0), Vector2f::new(2.0, 2.0));
        let b = Box2f::from_min_max(Vector2f::new(10.0, 10.0), Vector2f::new(12.0, 12.0));
        assert_eq!(a.difference(b), vec![a]);
    }

    #[test]
    fn difference_split_covers_remainder() {
        // 6x6 box minus a 2x2 box centered at (2..4, 2..4).
        let a = Box2f::from_min_max(Vector2f::new(0.0, 0.0), Vector2f::new(6.0, 6.0));
        let b = Box2f::from_min_max(Vector2f::new(2.0, 2.0), Vector2f::new(4.0, 4.0));
        let parts = a.difference(b);

        // No part may overlap b.
        for p in &parts {
            assert!(
                !p.intersects(&b),
                "difference slab overlaps the subtracted box"
            );
        }
        // Union area must equal a's area minus b's overlap (here 36 - 4 = 32).
        let total: f32 = parts
            .iter()
            .map(|p| (p.max.x - p.min.x) * (p.max.y - p.min.y))
            .sum();
        assert!((total - 32.0).abs() < 1e-4, "total area was {total}");
        // At most 4 slabs for a 2D difference.
        assert!(parts.len() <= 4);
    }
}
