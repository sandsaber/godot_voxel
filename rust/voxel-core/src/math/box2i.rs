//! Axis-aligned 2D box with integer coordinates.
//!
//! Faithful port of `util/math/box2i.h`. `position` is the min corner;
//! `position + size` is the exclusive max corner. The 2D analogue of
//! [`super::box3i::Box3i`]; cell traversal is YX order (the only order the C++
//! `for_each_cell_yx` uses).

use super::funcs;
use super::vector2::Vector2i;

/// Axis-aligned 2D box with integer coordinates. Matches C++ `Box2i`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Hash)]
#[repr(C)]
pub struct Box2i {
    pub position: Vector2i,
    pub size: Vector2i,
}

impl Box2i {
    pub const fn new(position: Vector2i, size: Vector2i) -> Self {
        Self { position, size }
    }

    pub const fn from_coords(ox: i32, oy: i32, sx: i32, sy: i32) -> Self {
        Self {
            position: Vector2i::new(ox, oy),
            size: Vector2i::new(sx, sy),
        }
    }

    /// Creates a box centered on a point, specifying half its size.
    /// Matches `from_center_extents`.
    pub fn from_center_extents(center: Vector2i, extents: Vector2i) -> Self {
        Self::new(center - extents, 2 * extents)
    }

    /// `p_max` is exclusive. Matches `from_min_max`.
    pub fn from_min_max(p_min: Vector2i, p_max: Vector2i) -> Self {
        Self::new(p_min, p_max - p_min)
    }

    /// Smallest box containing both `a` and `b`. Matches `get_bounding_box`.
    pub fn get_bounding_box(a: Box2i, b: Box2i) -> Self {
        let position = Vector2i::new(
            funcs::min(a.position.x, b.position.x),
            funcs::min(a.position.y, b.position.y),
        );
        let max_a = a.position + a.size;
        let max_b = b.position + b.size;
        let size = Vector2i::new(
            funcs::max(max_a.x, max_b.x) - position.x,
            funcs::max(max_a.y, max_b.y) - position.y,
        );
        Self::new(position, size)
    }

    #[inline]
    pub fn contains_point(&self, p: Vector2i) -> bool {
        let end = self.position + self.size;
        p.x >= self.position.x && p.y >= self.position.y && p.x < end.x && p.y < end.y
    }

    #[inline]
    pub fn contains_box(&self, other: Box2i) -> bool {
        let other_end = other.position + other.size;
        let end = self.position + self.size;
        other.position.x >= self.position.x
            && other.position.y >= self.position.y
            && other_end.x <= end.x
            && other_end.y <= end.y
    }

    #[inline]
    pub fn encloses(&self, other: Box2i) -> bool {
        self.position.x <= other.position.x
            && self.position.y <= other.position.y
            && self.position.x + self.size.x >= other.position.x + other.size.x
            && self.position.y + self.size.y >= other.position.y + other.size.y
    }

    #[inline]
    pub fn intersects(&self, other: &Box2i) -> bool {
        if self.position.x >= other.position.x + other.size.x {
            return false;
        }
        if self.position.y >= other.position.y + other.size.y {
            return false;
        }
        if other.position.x >= self.position.x + self.size.x {
            return false;
        }
        if other.position.y >= self.position.y + self.size.y {
            return false;
        }
        true
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size.x <= 0 || self.size.y <= 0
    }

    #[inline]
    pub fn padded(&self, m: i32) -> Box2i {
        Box2i::from_coords(
            self.position.x - m,
            self.position.y - m,
            self.size.x + 2 * m,
            self.size.y + 2 * m,
        )
    }

    /// Higher step-size coordinate, rounding **outwards**. Matches `downscaled`.
    pub fn downscaled(&self, step_size: i32) -> Box2i {
        let position = self.position.floordiv_scalar(step_size);
        let max_pos = (self.position + self.size - Vector2i::splat(1)).floordiv_scalar(step_size);
        let size = max_pos - position + Vector2i::splat(1);
        Box2i::new(position, size)
    }

    /// Higher step-size coordinate, rounding **inwards** (may be empty).
    /// Matches `downscaled_inner`.
    pub fn downscaled_inner(&self, step_size: i32) -> Box2i {
        let lo = self.position.ceildiv_scalar(step_size);
        let hi = (self.position + self.size).floordiv_scalar(step_size);
        Box2i::from_min_max(lo, hi)
    }

    pub fn snapped(&self, step: i32) -> Box2i {
        let mut r = self.downscaled(step);
        r.position *= step;
        r.size *= step;
        r
    }

    /// Clip a 1D range `[pos, pos+size)` into `[lim_pos, lim_pos+lim_size)`.
    /// Mirrors the static `clip_range` shared with `Box3i` (see [`funcs::clip_range`]).
    fn clip_range(pos: &mut i32, size: &mut i32, lim_pos: i32, lim_size: i32) {
        funcs::clip_range(pos, size, lim_pos, lim_size);
    }

    pub fn clip(&mut self, lim: Box2i) {
        Self::clip_range(
            &mut self.position.x,
            &mut self.size.x,
            lim.position.x,
            lim.size.x,
        );
        Self::clip_range(
            &mut self.position.y,
            &mut self.size.y,
            lim.position.y,
            lim.size.y,
        );
    }

    pub fn clipped(&self, lim: Box2i) -> Box2i {
        let mut copy = *self;
        copy.clip(lim);
        copy
    }

    /// Grow to also contain `other`. Matches `merge_with`.
    pub fn merge_with(&mut self, other: Box2i) {
        let min_pos = Vector2i::new(
            funcs::min(self.position.x, other.position.x),
            funcs::min(self.position.y, other.position.y),
        );
        let max_pos = Vector2i::new(
            funcs::max(
                self.position.x + self.size.x,
                other.position.x + other.size.x,
            ),
            funcs::max(
                self.position.y + self.size.y,
                other.position.y + other.size.y,
            ),
        );
        self.position = min_pos;
        self.size = max_pos - min_pos;
    }

    /// Subtract `b` from `self`, returning the (up to 4) boxes covering the
    /// remaining area. Matches `difference` / `difference_to_vec`.
    pub fn difference(&self, b: Box2i) -> Vec<Box2i> {
        let mut out = Vec::new();
        if !self.intersects(&b) {
            out.push(*self);
            return out;
        }

        let mut a = *self;
        let mut a_min = a.position;
        let mut a_max = a.position + a.size;
        let b_min = b.position;
        let b_max = b.position + b.size;

        if a_min.x < b_min.x {
            let rect_size = Vector2i::new(b_min.x - a_min.x, a.size.y);
            out.push(Box2i::new(a_min, rect_size));
            a_min.x = b_min.x;
            a.position.x = b.position.x;
            a.size.x = a_max.x - a_min.x;
        }
        if a_min.y < b_min.y {
            let rect_size = Vector2i::new(a.size.x, b_min.y - a_min.y);
            out.push(Box2i::new(a_min, rect_size));
            a_min.y = b_min.y;
            a.position.y = b.position.y;
            a.size.y = a_max.y - a_min.y;
        }
        if a_max.x > b_max.x {
            let rect_pos = Vector2i::new(b_max.x, a_min.y);
            let rect_size = Vector2i::new(a_max.x - b_max.x, a.size.y);
            out.push(Box2i::new(rect_pos, rect_size));
            a_max.x = b_max.x;
            a.size.x = a_max.x - a_min.x;
        }
        if a_max.y > b_max.y {
            let rect_pos = Vector2i::new(a_min.x, b_max.y);
            let rect_size = Vector2i::new(a.size.x, a_max.y - b_max.y);
            out.push(Box2i::new(rect_pos, rect_size));
        }
        out
    }

    /// Iterator over every cell position, **YX** order (y outer, x inner).
    /// Matches `for_each_cell_yx`.
    pub fn iter_cells(&self) -> impl Iterator<Item = Vector2i> + '_ {
        let start = self.position;
        let end = self.position + self.size;
        (start.y..end.y).flat_map(move |y| (start.x..end.x).map(move |x| Vector2i::new(x, y)))
    }

    /// True if every cell satisfies `predicate`. YX order; short-circuits.
    pub fn all_cells_match<P: Fn(Vector2i) -> bool>(&self, predicate: P) -> bool {
        self.iter_cells().all(predicate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_point_and_box() {
        let b = Box2i::from_coords(0, 0, 4, 4);
        assert!(b.contains_point(Vector2i::new(0, 0)));
        assert!(b.contains_point(Vector2i::new(3, 3)));
        assert!(!b.contains_point(Vector2i::new(4, 0))); // exclusive max
        assert!(b.contains_box(Box2i::from_coords(1, 1, 2, 2)));
        assert!(!b.contains_box(Box2i::from_coords(3, 0, 2, 1)));
    }

    #[test]
    fn intersects_and_encloses() {
        let a = Box2i::from_coords(0, 0, 4, 4);
        assert!(a.intersects(&Box2i::from_coords(2, 2, 4, 4)));
        assert!(!a.intersects(&Box2i::from_coords(4, 0, 2, 2)));
        assert!(a.encloses(Box2i::from_coords(0, 0, 4, 4)));
        assert!(!a.encloses(Box2i::from_coords(0, 0, 5, 4)));
    }

    #[test]
    fn ctors() {
        let c = Box2i::from_center_extents(Vector2i::new(10, 10), Vector2i::new(2, 2));
        assert_eq!(c.position, Vector2i::new(8, 8));
        assert_eq!(c.size, Vector2i::new(4, 4));

        let mm = Box2i::from_min_max(Vector2i::new(1, 1), Vector2i::new(5, 5));
        assert_eq!(mm.size, Vector2i::new(4, 4));

        let bb = Box2i::get_bounding_box(
            Box2i::from_coords(0, 0, 2, 2),
            Box2i::from_coords(5, 5, 2, 2),
        );
        assert_eq!(bb.position, Vector2i::zero());
        assert_eq!(bb.size, Vector2i::new(7, 7));
    }

    #[test]
    fn padded_clip_merge() {
        let b = Box2i::from_coords(0, 0, 4, 4);
        assert_eq!(b.padded(1).size, Vector2i::new(6, 6));

        let mut c = Box2i::from_coords(-2, 0, 10, 4);
        c.clip(Box2i::from_coords(0, 0, 4, 4));
        assert_eq!(c.position, Vector2i::zero());
        assert_eq!(c.size, Vector2i::new(4, 4));

        let mut a = Box2i::from_coords(0, 0, 2, 2);
        a.merge_with(Box2i::from_coords(4, 4, 2, 2));
        assert_eq!(a.size, Vector2i::new(6, 6));
    }

    #[test]
    fn difference_split() {
        let a = Box2i::from_coords(0, 0, 4, 4);
        let b = Box2i::from_coords(1, 1, 2, 2);
        let d = a.difference(b);
        let total: i64 = d
            .iter()
            .map(|x| (x.size.x as i64) * (x.size.y as i64))
            .sum();
        assert_eq!(total, 4 * 4 - 2 * 2);
        for r in &d {
            assert!(
                !r.intersects(&b),
                "difference slab overlaps the subtracted box"
            );
        }
    }

    #[test]
    fn difference_disjoint_returns_self() {
        let a = Box2i::from_coords(0, 0, 2, 2);
        let b = Box2i::from_coords(10, 10, 1, 1);
        assert_eq!(a.difference(b), vec![a]);
    }

    #[test]
    fn iter_cells_yx_order_and_count() {
        let b = Box2i::from_coords(0, 0, 2, 3);
        let cells: Vec<_> = b.iter_cells().collect();
        assert_eq!(cells.len(), 6);
        // YX: x is innermost.
        assert_eq!(cells[0], Vector2i::new(0, 0));
        assert_eq!(cells[1], Vector2i::new(1, 0));
        assert_eq!(cells[2], Vector2i::new(0, 1));
    }

    #[test]
    fn all_cells_match_short_circuits() {
        let b = Box2i::from_coords(0, 0, 3, 3);
        assert!(b.all_cells_match(|p| p.x < 3));
        assert!(!b.all_cells_match(|p| p.x < 2));
    }

    #[test]
    fn downscaled_rounds_outward() {
        let b = Box2i::from_coords(0, 0, 5, 1);
        let d = b.downscaled(2);
        assert_eq!(d.position, Vector2i::zero());
        assert_eq!(d.size, Vector2i::new(3, 1));
    }

    #[test]
    fn is_empty_when_any_axis_nonpositive() {
        assert!(Box2i::from_coords(0, 0, 0, 4).is_empty());
        assert!(!Box2i::from_coords(0, 0, 1, 1).is_empty());
    }
}
