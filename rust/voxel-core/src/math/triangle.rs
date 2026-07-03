//! Triangle geometry helpers.
//!
//! Ported from `util/math/triangle.h`. Covers point-in-triangle tests, area,
//! barycentric coordinates, and ray-triangle intersection (Möller–Trumbore),
//! including a "baked" variant that precomputes the direction-dependent part
//! for repeated raycasts sharing the same direction.

use super::funcs;
use super::vector2::Vector2f;
use super::vector3::{math as v3math, Vector3d, Vector3f};
use crate::math::vector2::math as v2math;

/// Which side of the half-open line `p_from + t*p_dir` (t > 0) a ray-triangle
/// test landed on. Matches `TriangleIntersectionResult::Case`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriangleIntersectionCase {
    /// The ray hits the triangle (at `distance` along the direction).
    Intersection,
    /// The ray is parallel to the triangle plane (will never hit).
    Parallel,
    /// The ray misses the triangle.
    NoIntersection,
}

/// Result of [`ray_intersects_triangle`]. `distance` is the parametric distance
/// `t` along the ray direction (i.e. hit point = `from + dir * distance`).
/// Matches `TriangleIntersectionResult`; the distance field is `f64` in C++.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriangleIntersectionResult {
    pub case_id: TriangleIntersectionCase,
    pub distance: f64,
}

impl TriangleIntersectionResult {
    const fn intersection(t: f64) -> Self {
        Self {
            case_id: TriangleIntersectionCase::Intersection,
            distance: t,
        }
    }
    const fn parallel() -> Self {
        Self {
            case_id: TriangleIntersectionCase::Parallel,
            distance: -1.0,
        }
    }
    const fn no_intersection() -> Self {
        Self {
            case_id: TriangleIntersectionCase::NoIntersection,
            distance: -1.0,
        }
    }
}

/// Float version of `Geometry::is_point_in_triangle`. Returns true if point `s`
/// lies inside (or on the edge of) triangle `a,b,c`. Matches the C++ overload
/// taking four `Vector2f`.
#[inline]
pub fn is_point_in_triangle(s: Vector2f, a: Vector2f, b: Vector2f, c: Vector2f) -> bool {
    let an = a - s;
    let bn = b - s;
    let cn = c - s;

    let orientation = v2math::cross(an, bn) > 0.0;

    if (v2math::cross(bn, cn) > 0.0) != orientation {
        return false;
    }

    (v2math::cross(cn, an) > 0.0) == orientation
}

/// Twice the squared area of triangle `p0,p1,p2` (i.e. `|cross(p1-p0, p2-p0)|²`).
/// Cheap degeneracy check avoiding `sqrt`. Matches `get_triangle_area_squared_x2`.
#[inline]
pub fn get_triangle_area_squared_x2(p0: Vector3f, p1: Vector3f, p2: Vector3f) -> f32 {
    let p01 = p1 - p0;
    let p02 = p2 - p0;
    let c = v3math::cross(p01, p02);
    v3math::length_squared(c)
}

/// True if the triangle's doubled squared area is below `epsilon_squared`.
/// Matches `is_triangle_degenerate_approx`.
#[inline]
pub fn is_triangle_degenerate_approx(
    p0: Vector3f,
    p1: Vector3f,
    p2: Vector3f,
    epsilon_squared: f32,
) -> bool {
    get_triangle_area_squared_x2(p0, p1, p2) < epsilon_squared
}

/// Area of triangle `p0,p1,p2` using a single `sqrt` (parallelogram cross / 2).
/// Matches the templated `get_triangle_area<T>` for `T = f32`.
#[inline]
pub fn get_triangle_area_f32(p0: Vector3f, p1: Vector3f, p2: Vector3f) -> f32 {
    let p01 = p1 - p0;
    let p02 = p2 - p0;
    let c = v3math::cross(p01, p02);
    0.5 * v3math::length(c)
}

/// Barycentric coordinates of point `p` w.r.t. triangle `p0,p1,p2` (2D).
/// Returns `(u, v, w)` with `u` at `p0`, `v` at `p1`, `w` at `p2`, and
/// `u + v + w == 1`. If the triangle is degenerate (zero denominator) the first
/// two weights are 0 and the third is 1. Matches `get_triangle_barycentric_coordinates`.
#[inline]
pub fn get_triangle_barycentric_coordinates(
    p0: Vector2f,
    p1: Vector2f,
    p2: Vector2f,
    p: Vector2f,
) -> Vector3f {
    let den = (p1.y - p2.y) * (p0.x - p2.x) + (p2.x - p1.x) * (p0.y - p2.y);
    let mut weights = Vector3f::zero();
    if !funcs::is_zero_approx(den) {
        weights.x = ((p1.y - p2.y) * (p.x - p2.x) + (p2.x - p1.x) * (p.y - p2.y)) / den;
        weights.y = ((p2.y - p0.y) * (p.x - p2.x) + (p0.x - p2.x) * (p.y - p2.y)) / den;
    }
    weights.z = 1.0 - weights.x - weights.y;
    weights
}

/// Random barycentric coordinates for uniform sampling inside a triangle, given
/// two uniform `[0,1]` random numbers. Matches `get_triangle_random_barycentric`.
/// See <https://stackoverflow.com/q/47410054>.
#[inline]
pub fn get_triangle_random_barycentric(rand1: f32, rand2: f32) -> Vector3f {
    let s = funcs::abs_f32(rand1 - rand2);
    let t = 0.5 * (rand1 + rand2 - s);
    let u = 1.0 - s - t;
    Vector3f::new(s, t, u)
}

/// Barycentric blend `a*u + b*v + c*w` given weights `barycentric = (u,v,w)`.
/// Generic over `T` (works for `f32`, `Vector3f`, …). Matches `interpolate_triangle`.
#[inline]
pub fn interpolate_triangle<T>(a: T, b: T, c: T, barycentric: Vector3f) -> T
where
    T: Copy + core::ops::Mul<f32, Output = T> + core::ops::Add<Output = T>,
{
    a * barycentric.x + b * barycentric.y + c * barycentric.z
}

/// Ray–triangle intersection (Möller–Trumbore), `f32` precision.
/// `p_dir` need not be normalized; `distance` is in units of `p_dir`.
/// Matches the `Vector3f` overload of `ray_intersects_triangle`.
pub fn ray_intersects_triangle(
    p_from: Vector3f,
    p_dir: Vector3f,
    p_v0: Vector3f,
    p_v1: Vector3f,
    p_v2: Vector3f,
) -> TriangleIntersectionResult {
    let e1 = p_v1 - p_v0;
    let e2 = p_v2 - p_v0;
    let h = v3math::cross(p_dir, e2);
    let a = v3math::dot(e1, h);

    if funcs::abs_f32(a) < 0.00001 {
        return TriangleIntersectionResult::parallel();
    }

    let f = 1.0 / a;

    let s = p_from - p_v0;
    let u = f * v3math::dot(s, h);

    if !(0.0..=1.0).contains(&u) {
        return TriangleIntersectionResult::no_intersection();
    }

    let q = v3math::cross(s, e1);
    let v = f * v3math::dot(p_dir, q);

    if v < 0.0 || u + v > 1.0 {
        return TriangleIntersectionResult::no_intersection();
    }

    // At this stage we can compute t to find out where
    // the intersection point is on the line.
    let t = f * v3math::dot(e2, q);

    if t > 0.00001 {
        // ray intersection
        TriangleIntersectionResult::intersection(t as f64)
    } else {
        // This means that there is a line intersection but not a ray intersection.
        TriangleIntersectionResult::no_intersection()
    }
}

// 3D scalar products for `Vector3d` (the generic `math::` namespace in C++ is
// only instantiated for `Vector3f` in the current Rust port, so we compute the
// double-precision raycast locally — same arithmetic, just `f64`).
#[inline]
fn cross_d(a: Vector3d, b: Vector3d) -> Vector3d {
    Vector3d::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}
#[inline]
fn dot_d(a: Vector3d, b: Vector3d) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Ray–triangle intersection (Möller–Trumbore), `f64` precision.
/// Matches the `Vector3d` overload of `ray_intersects_triangle`.
pub fn ray_intersects_triangle_d(
    p_from: Vector3d,
    p_dir: Vector3d,
    p_v0: Vector3d,
    p_v1: Vector3d,
    p_v2: Vector3d,
) -> TriangleIntersectionResult {
    let e1 = p_v1 - p_v0;
    let e2 = p_v2 - p_v0;
    let h = cross_d(p_dir, e2);
    let a = dot_d(e1, h);

    if a.abs() < 0.000000001 {
        // Parallel test.
        return TriangleIntersectionResult::parallel();
    }

    let f = 1.0 / a;

    let s = p_from - p_v0;
    let u = f * dot_d(s, h);

    if !(0.0..=1.0).contains(&u) {
        return TriangleIntersectionResult::no_intersection();
    }

    let q = cross_d(s, e1);
    let v = f * dot_d(p_dir, q);

    if v < 0.0 || u + v > 1.0 {
        return TriangleIntersectionResult::no_intersection();
    }

    let t = f * dot_d(e2, q);

    if t > 0.000000001 {
        TriangleIntersectionResult::intersection(t)
    } else {
        TriangleIntersectionResult::no_intersection()
    }
}

/// Precomputed triangle + fixed ray direction, for repeated raycasts that share
/// the same direction (e.g. marching a beam through many triangles).
///
/// `bake` must be called once with the triangle vertices and direction; `intersect`
/// then only needs the ray origin. `p_dir` passed to `intersect` **must** be the
/// same value used in `bake`. Matches `BakedIntersectionTriangleForFixedDirection`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BakedIntersectionTriangleForFixedDirection {
    v0: Vector3f,
    e1: Vector3f, // v1 - v0
    e2: Vector3f, // v2 - v0
    h: Vector3f,
    f: f32,
}

impl BakedIntersectionTriangleForFixedDirection {
    /// Precompute the direction-dependent terms. Returns `false` if the ray is
    /// parallel to the triangle plane (it will never hit).
    pub fn bake(
        &mut self,
        p_v0: Vector3f,
        p_v1: Vector3f,
        p_v2: Vector3f,
        p_dir: Vector3f,
    ) -> bool {
        self.v0 = p_v0;
        self.e1 = p_v1 - self.v0;
        self.e2 = p_v2 - self.v0;
        self.h = v3math::cross(p_dir, self.e2);
        let a = v3math::dot(self.e1, self.h);
        if funcs::abs_f32(a) < 0.00001 {
            // Parallel, will never hit
            return false;
        }
        self.f = 1.0 / a;
        true
    }

    /// Intersect a ray (origin `p_from`, the direction given to `bake`) with the
    /// baked triangle. Note: `p_dir` must be the same value used in `bake`.
    pub fn intersect(&self, p_from: Vector3f, p_dir: Vector3f) -> TriangleIntersectionResult {
        let s = p_from - self.v0;
        let u = self.f * v3math::dot(s, self.h);

        if !(0.0..=1.0).contains(&u) {
            return TriangleIntersectionResult::no_intersection();
        }

        let q = v3math::cross(s, self.e1);
        let v = self.f * v3math::dot(p_dir, q);

        if v < 0.0 || u + v > 1.0 {
            return TriangleIntersectionResult::no_intersection();
        }

        let t = self.f * v3math::dot(self.e2, q);

        if t > 0.00001 {
            TriangleIntersectionResult::intersection(t as f64)
        } else {
            TriangleIntersectionResult::no_intersection()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_triangle() {
        let a = Vector2f::new(0.0, 0.0);
        let b = Vector2f::new(2.0, 0.0);
        let c = Vector2f::new(0.0, 2.0);
        // Centroid-ish point is inside.
        assert!(is_point_in_triangle(Vector2f::new(0.5, 0.5), a, b, c));
        // Far outside.
        assert!(!is_point_in_triangle(Vector2f::new(5.0, 5.0), a, b, c));
        // Outside across the hypotenuse.
        assert!(!is_point_in_triangle(Vector2f::new(1.5, 1.5), a, b, c));
    }

    #[test]
    fn area_and_degenerate() {
        let p0 = Vector3f::new(0.0, 0.0, 0.0);
        let p1 = Vector3f::new(2.0, 0.0, 0.0);
        let p2 = Vector3f::new(0.0, 2.0, 0.0);
        // Right triangle with legs 2,2 -> area = 2.0.
        assert!((get_triangle_area_f32(p0, p1, p2) - 2.0).abs() < 1e-5);
        // Doubled squared area = |cross|² ; cross=(0,0,4) -> 16.
        assert!((get_triangle_area_squared_x2(p0, p1, p2) - 16.0).abs() < 1e-5);

        // Collinear points are degenerate.
        let q1 = Vector3f::new(1.0, 1.0, 1.0);
        let q2 = Vector3f::new(2.0, 2.0, 2.0);
        let q3 = Vector3f::new(3.0, 3.0, 3.0);
        assert!(is_triangle_degenerate_approx(q1, q2, q3, 1e-6));
    }

    #[test]
    fn barycentric_round_trip() {
        let p0 = Vector2f::new(0.0, 0.0);
        let p1 = Vector2f::new(4.0, 0.0);
        let p2 = Vector2f::new(0.0, 4.0);
        // At vertex p0 -> weights (1,0,0).
        let w0 = get_triangle_barycentric_coordinates(p0, p1, p2, p0);
        assert!((w0.x - 1.0).abs() < 1e-5);
        assert!(w0.y.abs() < 1e-5);
        assert!((w0.z - 0.0).abs() < 1e-5);
        // Weights sum to 1.
        let wm = get_triangle_barycentric_coordinates(p0, p1, p2, Vector2f::new(1.0, 1.0));
        assert!((wm.x + wm.y + wm.z - 1.0).abs() < 1e-5);
        // interpolate_triangle with those weights recovers the point (as a 2D blend).
        let reconstructed = interpolate_triangle(p0, p1, p2, wm);
        assert!((reconstructed.x - 1.0).abs() < 1e-5);
        assert!((reconstructed.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn random_barycentric_sums_to_one() {
        let w = get_triangle_random_barycentric(0.25, 0.75);
        assert!((w.x + w.y + w.z - 1.0).abs() < 1e-5);
        // All components non-negative for inputs in [0,1].
        assert!(w.x >= 0.0 && w.y >= 0.0 && w.z >= 0.0);
    }

    #[test]
    fn ray_hits_and_misses() {
        let v0 = Vector3f::new(-1.0, -1.0, 0.0);
        let v1 = Vector3f::new(1.0, -1.0, 0.0);
        let v2 = Vector3f::new(0.0, 1.0, 0.0);
        // Ray from +Z straight down through the centroid -> hits.
        let from = Vector3f::new(0.0, 0.0, 5.0);
        let dir = Vector3f::new(0.0, 0.0, -1.0);
        let hit = ray_intersects_triangle(from, dir, v0, v1, v2);
        assert_eq!(hit.case_id, TriangleIntersectionCase::Intersection);
        assert!((hit.distance - 5.0).abs() < 1e-4);

        // Ray far off to the side -> miss.
        let miss = ray_intersects_triangle(Vector3f::new(10.0, 10.0, 5.0), dir, v0, v1, v2);
        assert_eq!(miss.case_id, TriangleIntersectionCase::NoIntersection);

        // Ray parallel to the triangle plane (in-plane) -> parallel.
        let par = ray_intersects_triangle(
            Vector3f::new(0.0, 0.0, 0.0),
            Vector3f::new(1.0, 0.0, 0.0),
            v0,
            v1,
            v2,
        );
        assert_eq!(par.case_id, TriangleIntersectionCase::Parallel);
    }

    #[test]
    fn ray_double_precision_matches_float() {
        let v0 = Vector3d::new(-1.0, -1.0, 0.0);
        let v1 = Vector3d::new(1.0, -1.0, 0.0);
        let v2 = Vector3d::new(0.0, 1.0, 0.0);
        let from = Vector3d::new(0.0, 0.0, 5.0);
        let dir = Vector3d::new(0.0, 0.0, -1.0);
        let hit = ray_intersects_triangle_d(from, dir, v0, v1, v2);
        assert_eq!(hit.case_id, TriangleIntersectionCase::Intersection);
        assert!((hit.distance - 5.0).abs() < 1e-10);
    }

    #[test]
    fn baked_intersect_matches_direct() {
        let v0 = Vector3f::new(-1.0, -1.0, 0.0);
        let v1 = Vector3f::new(1.0, -1.0, 0.0);
        let v2 = Vector3f::new(0.0, 1.0, 0.0);
        let dir = Vector3f::new(0.0, 0.0, -1.0);

        let mut baked = BakedIntersectionTriangleForFixedDirection::default();
        assert!(baked.bake(v0, v1, v2, dir));

        let from = Vector3f::new(0.0, 0.0, 5.0);
        let direct = ray_intersects_triangle(from, dir, v0, v1, v2);
        let baked_hit = baked.intersect(from, dir);
        assert_eq!(direct.case_id, baked_hit.case_id);
        assert!((direct.distance - baked_hit.distance).abs() < 1e-5);

        // Parallel triangle cannot be baked.
        let par = baked.bake(v0, v1, v2, Vector3f::new(1.0, 0.0, 0.0));
        assert!(!par);
    }
}
