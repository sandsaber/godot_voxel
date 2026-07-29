//! Signed-distance-field primitives (scalar, `f32`).
//!
//! Ported from `util/math/sdf.h`. Only the **scalar** SDF functions are here;
//! the interval-arithmetic overloads (`IntervalT<T>`) are used by the graph
//! generator's bounds analysis and are ported in a later phase together with
//! `interval.h`. Reference for the formulas: <https://iquilezles.org/articles/distfunctions/>.
//!
//! Sign convention matches the engine: negative = inside the solid.

use super::funcs;
use super::vector2::{math as v2, Vector2f};
use super::vector3::{math as v3, Vector3f};

/// SDF of an axis-aligned box centered at the origin with half-extents `extents`.
/// Matches `sdf_box(Vector3T, Vector3T)`.
#[inline]
pub fn sdf_box(pos: Vector3f, extents: Vector3f) -> f32 {
    let d = Vector3f::new(
        funcs::abs_f32(pos.x),
        funcs::abs_f32(pos.y),
        funcs::abs_f32(pos.z),
    ) - extents;
    let outside = v3::length(Vector3f::new(
        funcs::max(d.x, 0.0),
        funcs::max(d.y, 0.0),
        funcs::max(d.z, 0.0),
    ));
    let inside = funcs::max3(d.x, d.y, d.z).min(0.0);
    outside + inside
}

/// SDF of a sphere: `distance(pos, center) - radius`. Matches `sdf_sphere`.
#[inline]
pub fn sdf_sphere(pos: Vector3f, center: Vector3f, radius: f32) -> f32 {
    v3::distance(pos, center) - radius
}

/// SDF of an infinite plane. `plane_d = dot(plane_normal, point_in_plane)`.
/// Matches `sdf_plane`.
#[inline]
pub fn sdf_plane(pos: Vector3f, plane_normal: Vector3f, plane_d: f32) -> f32 {
    v3::dot(pos, plane_normal) - plane_d
}

/// SDF of a torus around the Y axis with major radius `r0` and minor radius `r1`.
/// Matches `sdf_torus` (C++ takes x/y/z separately; here we take `pos`).
#[inline]
pub fn sdf_torus(pos: Vector3f, r0: f32, r1: f32) -> f32 {
    let q = Vector2f::new(v2::length(Vector2f::new(pos.x, pos.z)) - r0, pos.y);
    v2::length(q) - r1
}

/// Exact union of two SDFs (set minimum). Matches `sdf_union`.
#[inline]
pub fn sdf_union(a: f32, b: f32) -> f32 {
    funcs::min(a, b)
}

/// Subtract SDF `b` from SDF `a`. Matches `sdf_subtract`.
#[inline]
pub fn sdf_subtract(a: f32, b: f32) -> f32 {
    funcs::max(a, -b)
}

/// Polynomial smooth union. Matches `sdf_smooth_union`; `s` is the smoothness radius.
#[inline]
pub fn sdf_smooth_union(a: f32, b: f32, s: f32) -> f32 {
    let h = funcs::clamp(0.5 + 0.5 * (b - a) / s, 0.0, 1.0);
    funcs::lerp_f32(b, a, h) - s * h * (1.0 - h)
}

/// Polynomial smooth subtract (`b - a`). Matches `sdf_smooth_subtract`.
#[inline]
pub fn sdf_smooth_subtract(b: f32, a: f32, s: f32) -> f32 {
    let h = funcs::clamp(0.5 - 0.5 * (b + a) / s, 0.0, 1.0);
    funcs::lerp_f32(b, -a, h) + s * h * (1.0 - h)
}

/// SDF of a round cone between points `a`→`b` with radii `r1` (at `a`) and `r2` (at `b`).
/// Matches `sdf_round_cone`.
#[allow(clippy::excessive_precision)]
pub fn sdf_round_cone(p: Vector3f, a: Vector3f, b: Vector3f, r1: f32, r2: f32) -> f32 {
    // Sampling-independent setup (see also SdfRoundConePrecalc).
    let ba = b - a;
    let l2 = v3::dot(ba, ba);
    let rr = r1 - r2;
    let a2 = l2 - rr * rr;
    let il2 = 1.0 / l2;

    // Sampling-dependent.
    let pa = p - a;
    let y = v3::dot(pa, ba);
    let z = y - l2;
    let x2 = v3::length_squared(pa * il2 - ba * y);
    let y2 = y * y * l2;
    let z2 = z * z * l2;

    let k = funcs::sign_f32(rr) * rr * rr * x2;
    if funcs::sign_f32(z) * a2 * z2 > k {
        return funcs::sqrt_f32(x2 + z2) * il2 - r2;
    }
    if funcs::sign_f32(y) * a2 * y2 < k {
        return funcs::sqrt_f32(x2 + y2) * il2 - r1;
    }
    (funcs::sqrt_f32(x2 * a2 * il2) + y * rr) * il2 - r1
}

/// Precomputed round-cone parameters, for evaluating the same cone against many
/// points. Matches `SdfRoundConePrecalc`. Call [`SdfRoundConePrecalc::new`] once,
/// then [`SdfRoundConePrecalc::eval`] per sample.
#[derive(Debug, Clone, Copy)]
pub struct SdfRoundConePrecalc {
    a: Vector3f,
    ba: Vector3f,
    l2: f32,
    rr: f32,
    a2: f32,
    il2: f32,
    r1: f32,
    r2: f32,
}

impl SdfRoundConePrecalc {
    pub fn new(a: Vector3f, b: Vector3f, r1: f32, r2: f32) -> Self {
        let ba = b - a;
        let l2 = v3::dot(ba, ba);
        let rr = r1 - r2;
        let a2 = l2 - rr * rr;
        let il2 = 1.0 / l2;
        Self {
            a,
            ba,
            l2,
            rr,
            a2,
            il2,
            r1,
            r2,
        }
    }

    /// Signed distance from `p` to the cone.
    pub fn eval(&self, p: Vector3f) -> f32 {
        let pa = p - self.a;
        let y = v3::dot(pa, self.ba);
        let z = y - self.l2;
        let x2 = v3::length_squared(pa * self.il2 - self.ba * y);
        let y2 = y * y * self.l2;
        let z2 = z * z * self.l2;

        let k = funcs::sign_f32(self.rr) * self.rr * self.rr * x2;
        if funcs::sign_f32(z) * self.a2 * z2 > k {
            return funcs::sqrt_f32(x2 + z2) * self.il2 - self.r2;
        }
        if funcs::sign_f32(y) * self.a2 * y2 < k {
            return funcs::sqrt_f32(x2 + y2) * self.il2 - self.r1;
        }
        (funcs::sqrt_f32(x2 * self.a2 * self.il2) + y * self.rr) * self.il2 - self.r1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_signs() {
        let c = Vector3f::zero();
        // At the center, distance is -radius (inside).
        assert!((sdf_sphere(Vector3f::zero(), c, 5.0) - (-5.0)).abs() < 1e-5);
        // On the surface, ~0.
        assert!(sdf_sphere(Vector3f::new(5.0, 0.0, 0.0), c, 5.0).abs() < 1e-5);
        // Outside, positive.
        assert!(sdf_sphere(Vector3f::new(10.0, 0.0, 0.0), c, 5.0) > 0.0);
    }

    #[test]
    fn box_inside_outside() {
        // 2x2x2 box (extents 1): center is inside (negative), face surface ~0.
        let d_center = sdf_box(Vector3f::zero(), Vector3f::splat(1.0));
        assert!(d_center < 0.0);
        let d_face = sdf_box(Vector3f::new(1.0, 0.0, 0.0), Vector3f::splat(1.0));
        assert!(d_face.abs() < 1e-5);
        assert!(sdf_box(Vector3f::new(5.0, 0.0, 0.0), Vector3f::splat(1.0)) > 0.0);
    }

    #[test]
    fn plane_offset() {
        // Plane y=1 (normal (0,1,0), d=1): point (0,1,0) -> 0, point (0,3,0) -> 2.
        let n = Vector3f::new(0.0, 1.0, 0.0);
        assert!(sdf_plane(Vector3f::new(0.0, 1.0, 0.0), n, 1.0).abs() < 1e-5);
        assert!((sdf_plane(Vector3f::new(0.0, 3.0, 0.0), n, 1.0) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn csg_ops() {
        // Union is min.
        assert_eq!(sdf_union(-3.0, 5.0), -3.0);
        // Subtract: max(a, -b).
        assert_eq!(sdf_subtract(2.0, 4.0), 2.0); // max(2, -4) = 2
        assert_eq!(sdf_subtract(2.0, -1.0), 2.0); // max(2, 1) = 2
                                                  // Smooth union approaches plain min far from the junction.
        let far = sdf_smooth_union(-10.0, 10.0, 1.0);
        assert!((far - (-10.0)).abs() < 1e-3);
    }

    #[test]
    fn torus_surface() {
        // Torus r0=4 (ring radius), r1=1 (tube radius): point (5,0,0) sits one tube-radius
        // out from the ring circle (4,0,0), so it is exactly on the surface.
        let d = sdf_torus(Vector3f::new(5.0, 0.0, 0.0), 4.0, 1.0);
        assert!(d.abs() < 1e-4);
        // The ring center (4,0,0) is inside the tube by r1 -> -1.
        assert!((sdf_torus(Vector3f::new(4.0, 0.0, 0.0), 4.0, 1.0) - (-1.0)).abs() < 1e-4);
    }
}
