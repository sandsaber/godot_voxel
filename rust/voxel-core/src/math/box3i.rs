//! Axis-aligned 3D box with integer coordinates.
//!
//! Faithful port of `util/math/box3i.h`. The box is a `position` (min corner)
//! plus a `size`; the max corner `position + size` is **exclusive**. Used
//! pervasively for chunk/bounds arithmetic in storage and terrain.
//!
//! A few C++ template callbacks (`for_each_cell`, `difference`, …) become more
//! idiomatic here: cell traversal is exposed as iterators (`iter_cells`,
//! `iter_cells_zxy`), and `difference` returns a `Vec<Box3i>`. The traversal
//! *order* (ZYX / ZXY) is preserved exactly — downstream code and parity tests
//! rely on it.

use super::funcs;
use super::vector3::Vector3i;

/// Axis-aligned 3D box with integer coordinates. `position` is the min corner;
/// `position + size` is the exclusive max corner. Matches C++ `Box3i`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Box3i {
    pub position: Vector3i,
    pub size: Vector3i,
}

impl Box3i {
    pub const fn new(position: Vector3i, size: Vector3i) -> Self {
        Self { position, size }
    }

    pub const fn from_coords(ox: i32, oy: i32, oz: i32, sx: i32, sy: i32, sz: i32) -> Self {
        Self {
            position: Vector3i::new(ox, oy, oz),
            size: Vector3i::new(sx, sy, sz),
        }
    }

    /// Creates a box centered on a point, specifying half its size.
    /// Matches `from_center_extents`. (If you want the center to be a 1x1x1 box
    /// rather than a point, add 1 to `extents`.)
    pub fn from_center_extents(center: Vector3i, extents: Vector3i) -> Self {
        Self::new(center - extents, Vector3i::splat(2) * extents)
    }

    /// `p_max` is exclusive. Matches `from_min_max`.
    pub fn from_min_max(p_min: Vector3i, p_max: Vector3i) -> Self {
        Self::new(p_min, p_max - p_min)
    }

    /// Smallest box containing both `a` and `b`. Matches `get_bounding_box`.
    pub fn get_bounding_box(a: Box3i, b: Box3i) -> Self {
        let position = Vector3i::new(
            funcs::min(a.position.x, b.position.x),
            funcs::min(a.position.y, b.position.y),
            funcs::min(a.position.z, b.position.z),
        );
        let max_a = a.position + a.size;
        let max_b = b.position + b.size;
        let size = Vector3i::new(
            funcs::max(max_a.x, max_b.x) - position.x,
            funcs::max(max_a.y, max_b.y) - position.y,
            funcs::max(max_a.z, max_b.z) - position.z,
        );
        Self::new(position, size)
    }

    /// Inclusive-min / exclusive-max corner test. Matches `contains(Vector3i)`.
    #[inline]
    pub fn contains_point(&self, p: Vector3i) -> bool {
        let end = self.position + self.size;
        p.x >= self.position.x
            && p.y >= self.position.y
            && p.z >= self.position.z
            && p.x < end.x
            && p.y < end.y
            && p.z < end.z
    }

    /// True if `other` lies entirely inside `self`. Matches `contains(Box3i)`.
    #[inline]
    pub fn contains_box(&self, other: Box3i) -> bool {
        let other_end = other.position + other.size;
        let end = self.position + self.size;
        other.position.x >= self.position.x
            && other.position.y >= self.position.y
            && other.position.z >= self.position.z
            && other_end.x <= end.x
            && other_end.y <= end.y
            && other_end.z <= end.z
    }

    /// Same as [`contains_box`](Self::contains_box); matches the C++ method name.
    #[inline]
    pub fn encloses(&self, other: Box3i) -> bool {
        self.position.x <= other.position.x
            && self.position.y <= other.position.y
            && self.position.z <= other.position.z
            && self.position.x + self.size.x >= other.position.x + other.size.x
            && self.position.y + self.size.y >= other.position.y + other.size.y
            && self.position.z + self.size.z >= other.position.z + other.size.z
    }

    #[inline]
    pub fn intersects(&self, other: &Box3i) -> bool {
        if self.position.x >= other.position.x + other.size.x {
            return false;
        }
        if self.position.y >= other.position.y + other.size.y {
            return false;
        }
        if self.position.z >= other.position.z + other.size.z {
            return false;
        }
        if other.position.x >= self.position.x + self.size.x {
            return false;
        }
        if other.position.y >= self.position.y + self.size.y {
            return false;
        }
        if other.position.z >= self.position.z + self.size.z {
            return false;
        }
        true
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size.x <= 0 || self.size.y <= 0 || self.size.z <= 0
    }

    #[inline]
    pub fn padded(&self, m: i32) -> Box3i {
        Box3i::from_coords(
            self.position.x - m,
            self.position.y - m,
            self.position.z - m,
            self.size.x + 2 * m,
            self.size.y + 2 * m,
            self.size.z + 2 * m,
        )
    }

    #[inline]
    pub fn scaled(&self, scale: i32) -> Box3i {
        Box3i::new(self.position * scale, self.size * scale)
    }

    /// Higher step-size coordinate, rounding **outwards**. Matches `downscaled`.
    pub fn downscaled(&self, step_size: i32) -> Box3i {
        let position = Vector3i::new(
            funcs::floordiv(self.position.x, step_size),
            funcs::floordiv(self.position.y, step_size),
            funcs::floordiv(self.position.z, step_size),
        );
        let max_pos = Vector3i::new(
            funcs::floordiv(self.position.x + self.size.x - 1, step_size),
            funcs::floordiv(self.position.y + self.size.y - 1, step_size),
            funcs::floordiv(self.position.z + self.size.z - 1, step_size),
        );
        let size = max_pos - position + Vector3i::splat(1);
        Box3i::new(position, size)
    }

    /// Higher step-size coordinate, rounding **inwards** so the result is
    /// contained in the original (may be empty). Matches `downscaled_inner`.
    pub fn downscaled_inner(&self, step_size: i32) -> Box3i {
        let lo = Vector3i::new(
            funcs::ceildiv(self.position.x, step_size),
            funcs::ceildiv(self.position.y, step_size),
            funcs::ceildiv(self.position.z, step_size),
        );
        let hi = Vector3i::new(
            funcs::floordiv(self.position.x + self.size.x, step_size),
            funcs::floordiv(self.position.y + self.size.y, step_size),
            funcs::floordiv(self.position.z + self.size.z, step_size),
        );
        Box3i::from_min_max(lo, hi)
    }

    pub fn snapped(&self, step: i32) -> Box3i {
        let mut r = self.downscaled(step);
        r.position *= step;
        r.size *= step;
        r
    }

    pub fn clip(&mut self, lim: Box3i) {
        funcs::clip_range(
            &mut self.position.x,
            &mut self.size.x,
            lim.position.x,
            lim.size.x,
        );
        funcs::clip_range(
            &mut self.position.y,
            &mut self.size.y,
            lim.position.y,
            lim.size.y,
        );
        funcs::clip_range(
            &mut self.position.z,
            &mut self.size.z,
            lim.position.z,
            lim.size.z,
        );
    }

    pub fn clip_to_size(&mut self, lim_size: Vector3i) {
        funcs::clip_range(&mut self.position.x, &mut self.size.x, 0, lim_size.x);
        funcs::clip_range(&mut self.position.y, &mut self.size.y, 0, lim_size.y);
        funcs::clip_range(&mut self.position.z, &mut self.size.z, 0, lim_size.z);
    }

    pub fn clipped(&self, lim: Box3i) -> Box3i {
        let mut copy = *self;
        copy.clip(lim);
        copy
    }

    pub fn clipped_to_size(&self, lim_size: Vector3i) -> Box3i {
        let mut copy = *self;
        copy.clip_to_size(lim_size);
        copy
    }

    /// Grow to also contain `other`. Matches `merge_with`.
    pub fn merge_with(&mut self, other: Box3i) {
        let min_pos = Vector3i::new(
            funcs::min(self.position.x, other.position.x),
            funcs::min(self.position.y, other.position.y),
            funcs::min(self.position.z, other.position.z),
        );
        let max_pos = Vector3i::new(
            funcs::max(
                self.position.x + self.size.x,
                other.position.x + other.size.x,
            ),
            funcs::max(
                self.position.y + self.size.y,
                other.position.y + other.size.y,
            ),
            funcs::max(
                self.position.z + self.size.z,
                other.position.z + other.size.z,
            ),
        );
        self.position = min_pos;
        self.size = max_pos - min_pos;
    }

    /// Subtract `b` from `self`, returning the (up to 6) boxes that cover the
    /// remaining volume. Matches `difference` / `difference_to_vec`.
    pub fn difference(&self, b: Box3i) -> Vec<Box3i> {
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
            let rect_size = Vector3i::new(b_min.x - a_min.x, a.size.y, a.size.z);
            out.push(Box3i::new(a_min, rect_size));
            a_min.x = b_min.x;
            a.position.x = b.position.x;
            a.size.x = a_max.x - a_min.x;
        }
        if a_min.y < b_min.y {
            let rect_size = Vector3i::new(a.size.x, b_min.y - a_min.y, a.size.z);
            out.push(Box3i::new(a_min, rect_size));
            a_min.y = b_min.y;
            a.position.y = b.position.y;
            a.size.y = a_max.y - a_min.y;
        }
        if a_min.z < b_min.z {
            let rect_size = Vector3i::new(a.size.x, a.size.y, b_min.z - a_min.z);
            out.push(Box3i::new(a_min, rect_size));
            a_min.z = b_min.z;
            a.position.z = b.position.z;
            a.size.z = a_max.z - a_min.z;
        }
        if a_max.x > b_max.x {
            let rect_pos = Vector3i::new(b_max.x, a_min.y, a_min.z);
            let rect_size = Vector3i::new(a_max.x - b_max.x, a.size.y, a.size.z);
            out.push(Box3i::new(rect_pos, rect_size));
            a_max.x = b_max.x;
            a.size.x = a_max.x - a_min.x;
        }
        if a_max.y > b_max.y {
            let rect_pos = Vector3i::new(a_min.x, b_max.y, a_min.z);
            let rect_size = Vector3i::new(a.size.x, a_max.y - b_max.y, a.size.z);
            out.push(Box3i::new(rect_pos, rect_size));
            a_max.y = b_max.y;
            a.size.y = a_max.y - a_min.y;
        }
        if a_max.z > b_max.z {
            let rect_pos = Vector3i::new(a_min.x, a_min.y, b_max.z);
            let rect_size = Vector3i::new(a.size.x, a.size.y, a_max.z - b_max.z);
            out.push(Box3i::new(rect_pos, rect_size));
        }
        out
    }

    /// Iterator over every cell position, **ZYX** order (z outer, x inner).
    /// Matches `for_each_cell`. Yields `size.x * size.y * size.z` positions.
    pub fn iter_cells(&self) -> impl Iterator<Item = Vector3i> + '_ {
        let start = self.position;
        let end = self.position + self.size;
        (start.z..end.z).flat_map(move |z| {
            (start.y..end.y)
                .flat_map(move |y| (start.x..end.x).map(move |x| Vector3i::new(x, y, z)))
        })
    }

    /// Iterator over every cell position, **ZXY** order (z outer, y inner).
    /// Matches `for_each_cell_zxy`.
    pub fn iter_cells_zxy(&self) -> impl Iterator<Item = Vector3i> + '_ {
        let start = self.position;
        let end = self.position + self.size;
        (start.z..end.z).flat_map(move |z| {
            (start.x..end.x)
                .flat_map(move |x| (start.y..end.y).map(move |y| Vector3i::new(x, y, z)))
        })
    }

    /// True if every cell satisfies `predicate`. ZYX order; short-circuits.
    /// Matches `all_cells_match`.
    pub fn all_cells_match<P: Fn(Vector3i) -> bool>(&self, predicate: P) -> bool {
        self.iter_cells().all(predicate)
    }

    /// Call `f` on each cell on the box's surface (inner outline). Order is not
    /// guaranteed (matches `for_inner_outline`). Returns the count of cells.
    pub fn for_inner_outline<F: FnMut(Vector3i)>(&self, mut f: F) {
        let mut min_pos = self.position;
        let mut max_pos = self.position + self.size;

        // Top and bottom (y faces).
        for z in min_pos.z..max_pos.z {
            for x in min_pos.x..max_pos.x {
                f(Vector3i::new(x, min_pos.y, z));
                f(Vector3i::new(x, max_pos.y - 1, z));
            }
        }
        // Exclude top/bottom rows from the sides iterated next.
        min_pos.y += 1;
        max_pos.y -= 1;

        // Z faces.
        for x in min_pos.x..max_pos.x {
            for y in min_pos.y..max_pos.y {
                f(Vector3i::new(x, y, min_pos.z));
                f(Vector3i::new(x, y, max_pos.z - 1));
            }
        }
        // Exclude edges of the Z faces.
        min_pos.z += 1;
        max_pos.z -= 1;

        // X faces.
        for z in min_pos.z..max_pos.z {
            for y in min_pos.y..max_pos.y {
                f(Vector3i::new(min_pos.x, y, z));
                f(Vector3i::new(max_pos.x - 1, y, z));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_point_and_box() {
        let b = Box3i::from_coords(0, 0, 0, 4, 4, 4);
        assert!(b.contains_point(Vector3i::new(0, 0, 0)));
        assert!(b.contains_point(Vector3i::new(3, 3, 3)));
        assert!(!b.contains_point(Vector3i::new(4, 0, 0))); // exclusive max
        assert!(!b.contains_point(Vector3i::new(-1, 0, 0)));
        assert!(b.contains_box(Box3i::from_coords(1, 1, 1, 2, 2, 2)));
        assert!(!b.contains_box(Box3i::from_coords(3, 0, 0, 2, 1, 1)));
    }

    #[test]
    fn intersects_and_encloses() {
        let a = Box3i::from_coords(0, 0, 0, 4, 4, 4);
        assert!(a.intersects(&Box3i::from_coords(2, 2, 2, 4, 4, 4)));
        assert!(!a.intersects(&Box3i::from_coords(4, 0, 0, 2, 2, 2))); // touches edge, not overlap
        assert!(a.encloses(Box3i::from_coords(0, 0, 0, 4, 4, 4)));
        assert!(!a.encloses(Box3i::from_coords(0, 0, 0, 5, 4, 4)));
    }

    #[test]
    fn ctors() {
        let c = Box3i::from_center_extents(Vector3i::new(10, 10, 10), Vector3i::new(2, 2, 2));
        assert_eq!(c.position, Vector3i::new(8, 8, 8));
        assert_eq!(c.size, Vector3i::new(4, 4, 4));

        let mm = Box3i::from_min_max(Vector3i::new(1, 1, 1), Vector3i::new(5, 5, 5));
        assert_eq!(mm.size, Vector3i::new(4, 4, 4));

        let bb = Box3i::get_bounding_box(
            Box3i::from_coords(0, 0, 0, 2, 2, 2),
            Box3i::from_coords(5, 5, 5, 2, 2, 2),
        );
        assert_eq!(bb.position, Vector3i::zero());
        assert_eq!(bb.size, Vector3i::new(7, 7, 7));
    }

    #[test]
    fn padded_scaled_snapped() {
        let b = Box3i::from_coords(0, 0, 0, 4, 4, 4);
        assert_eq!(b.padded(1).size, Vector3i::new(6, 6, 6));
        assert_eq!(b.padded(1).position, Vector3i::new(-1, -1, -1));
        assert_eq!(b.scaled(2), Box3i::from_coords(0, 0, 0, 8, 8, 8));
        // snapped downscales then re-scales; for an already-aligned box it's identity-ish.
        let s = b.snapped(4);
        assert_eq!(s.position, Vector3i::zero());
    }

    #[test]
    fn clip_and_clipped() {
        let mut b = Box3i::from_coords(-2, 0, 0, 10, 4, 4);
        b.clip(Box3i::from_coords(0, 0, 0, 4, 4, 4));
        assert_eq!(b.position, Vector3i::zero());
        assert_eq!(b.size, Vector3i::new(4, 4, 4));

        let c = Box3i::from_coords(0, 0, 0, 10, 4, 4).clipped_to_size(Vector3i::new(4, 4, 4));
        assert_eq!(c.size, Vector3i::new(4, 4, 4));
    }

    #[test]
    fn merge_with_grows() {
        let mut a = Box3i::from_coords(0, 0, 0, 2, 2, 2);
        a.merge_with(Box3i::from_coords(4, 4, 4, 2, 2, 2));
        assert_eq!(a.position, Vector3i::zero());
        assert_eq!(a.size, Vector3i::new(6, 6, 6));
    }

    #[test]
    fn difference_disjoint_returns_self() {
        let a = Box3i::from_coords(0, 0, 0, 2, 2, 2);
        let b = Box3i::from_coords(10, 10, 10, 1, 1, 1);
        let d = a.difference(b);
        assert_eq!(d, vec![a]);
    }

    #[test]
    fn difference_split() {
        // Subtracting a central box leaves up to 6 slabs.
        let a = Box3i::from_coords(0, 0, 0, 4, 4, 4);
        let b = Box3i::from_coords(1, 1, 1, 2, 2, 2);
        let d = a.difference(b);
        // The union of the resulting boxes must equal a's volume minus b's overlap.
        let total: i64 = d
            .iter()
            .map(|x| (x.size.x as i64) * (x.size.y as i64) * (x.size.z as i64))
            .sum();
        assert_eq!(total, 4 * 4 * 4 - 2 * 2 * 2);
        // No result box may overlap b.
        for r in &d {
            assert!(
                !r.intersects(&b),
                "difference slab overlaps the subtracted box"
            );
        }
    }

    #[test]
    fn iter_cells_order_and_count() {
        let b = Box3i::from_coords(0, 0, 0, 2, 2, 2);
        let zyx: Vec<_> = b.iter_cells().collect();
        assert_eq!(zyx.len(), 8);
        // ZYX: x is innermost.
        assert_eq!(zyx[0], Vector3i::new(0, 0, 0));
        assert_eq!(zyx[1], Vector3i::new(1, 0, 0));
        assert_eq!(zyx[2], Vector3i::new(0, 1, 0));

        let zxy: Vec<_> = b.iter_cells_zxy().collect();
        assert_eq!(zxy.len(), 8);
        // ZXY: y is innermost.
        assert_eq!(zxy[0], Vector3i::new(0, 0, 0));
        assert_eq!(zxy[1], Vector3i::new(0, 1, 0));
        assert_eq!(zxy[2], Vector3i::new(1, 0, 0));
    }

    #[test]
    fn all_cells_match_short_circuits() {
        let b = Box3i::from_coords(0, 0, 0, 3, 3, 3);
        assert!(b.all_cells_match(|p| p.x < 3));
        assert!(!b.all_cells_match(|p| p.x < 2));
    }

    #[test]
    fn inner_outline_surface_count() {
        // A 4x4x4 solid box has surface = 4^3 - 2^3 = 64 - 8 = 56 cells.
        let b = Box3i::from_coords(0, 0, 0, 4, 4, 4);
        let mut count = 0usize;
        b.for_inner_outline(|_| count += 1);
        assert_eq!(count, 56);
    }

    #[test]
    fn downscaled_rounds_outward() {
        // 0..5 (5 cells) at step 2 -> floor(0/2)=0 .. floor(4/2)=2, size 3 (covers 0..6).
        let b = Box3i::from_coords(0, 0, 0, 5, 1, 1);
        let d = b.downscaled(2);
        assert_eq!(d.position, Vector3i::zero());
        assert_eq!(d.size, Vector3i::new(3, 1, 1));
    }

    #[test]
    fn is_empty_when_any_axis_nonpositive() {
        assert!(Box3i::from_coords(0, 0, 0, 0, 4, 4).is_empty());
        assert!(Box3i::from_coords(0, 0, 0, 4, -1, 4).is_empty());
        assert!(!Box3i::from_coords(0, 0, 0, 1, 1, 1).is_empty());
    }
}
