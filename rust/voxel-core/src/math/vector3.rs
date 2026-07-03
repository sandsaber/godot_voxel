//! 3D vector types.
//!
//! Ported from `util/math/vector3t.h` (template `Vector3T<T>`) and
//! `util/math/vector3f.h` (free math functions on `Vector3f = Vector3T<f32>`).
//!
//! In C++ the design splits "struct + operators" (vector3t.h) from "free math
//! functions" (vector3f.h), to mirror shader-style overloading. We keep the
//! same conceptual split: this module exposes `Vector3T<T>` with operators,
//! plus free `math::*` functions; type aliases (`Vector3f`, `Vector3i`) are
//! defined at the bottom.

use super::constants::Axis;
use super::funcs;

/// Generic 3D vector. Matches `Vector3T<T>` in C++: plain fields + operators.
///
/// Stored as `#[repr(C)]` so it can be passed across FFI / copied into GPU
/// buffers with the exact same memory layout as the C++ `union { struct {x,y,z}; T coords[3]; }`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Vector3T<T: Copy> {
    pub x: T,
    pub y: T,
    pub z: T,
}

impl<T: Copy> Vector3T<T> {
    pub const fn new(x: T, y: T, z: T) -> Self {
        Self { x, y, z }
    }

    /// Construct from a single scalar broadcast to all axes (matches the
    /// `explicit Vector3T(T)` C++ ctor — note: we keep it non-`pub(crate)` to
    /// avoid accidental implicit conversions, callers must name it).
    pub const fn splat(v: T) -> Self {
        Self { x: v, y: v, z: v }
    }

    #[inline]
    pub fn axis(&self, axis: Axis) -> T {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
            Axis::Z => self.z,
        }
    }

    #[inline]
    pub fn axis_mut(&mut self, axis: Axis) -> &mut T {
        match axis {
            Axis::X => &mut self.x,
            Axis::Y => &mut self.y,
            Axis::Z => &mut self.z,
        }
    }

    /// Index access with runtime bounds check (debug). Mirrors `operator[]`.
    #[inline]
    pub fn get(&self, i: usize) -> T {
        debug_assert!(i < 3);
        match i {
            0 => self.x,
            1 => self.y,
            2 => self.z,
            _ => unsafe { core::hint::unreachable_unchecked() },
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: T) {
        debug_assert!(i < 3);
        match i {
            0 => self.x = v,
            1 => self.y = v,
            2 => self.z = v,
            _ => unsafe { core::hint::unreachable_unchecked() },
        }
    }
}

// `v[i]` indexing (matches `operator[]` / the `get`+`set` pair above).
impl<T: Copy> core::ops::Index<usize> for Vector3T<T> {
    type Output = T;
    #[inline]
    fn index(&self, i: usize) -> &T {
        debug_assert!(i < 3);
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            _ => panic!("Vector3 index out of range"),
        }
    }
}

impl<T: Copy> core::ops::IndexMut<usize> for Vector3T<T> {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut T {
        debug_assert!(i < 3);
        match i {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            _ => panic!("Vector3 index out of range"),
        }
    }
}

// ---- Swizzles (match vector3t.h xyz/zyx/zxy/yzx) ----

impl<T: Copy> Vector3T<T> {
    #[inline]
    pub fn xyz(self) -> Self { self }

    #[inline]
    pub fn zyx(self) -> Self { Self { x: self.z, y: self.y, z: self.x } }

    #[inline]
    pub fn zxy(self) -> Self { Self { x: self.z, y: self.x, z: self.y } }

    #[inline]
    pub fn yzx(self) -> Self { Self { x: self.y, y: self.z, z: self.x } }
}

// ---- Operator overloading via std::ops ----
// We impl for the concrete numeric cases we actually need rather than
// trying to be fully generic; that keeps the trait bounds tractable and
// matches how the C++ template is instantiated (Vector3f / Vector3i).

macro_rules! impl_vector_ops {
    ($t:ty, $zero:expr) => {
        impl Vector3T<$t> {
            #[inline]
            pub fn zero() -> Self { Self::splat($zero) }
        }

        impl core::ops::Add for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self { Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z) }
        }

        impl core::ops::AddAssign for Vector3T<$t> {
            #[inline]
            fn add_assign(&mut self, rhs: Self) {
                self.x += rhs.x; self.y += rhs.y; self.z += rhs.z;
            }
        }

        impl core::ops::Sub for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: Self) -> Self { Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z) }
        }

        impl core::ops::SubAssign for Vector3T<$t> {
            #[inline]
            fn sub_assign(&mut self, rhs: Self) {
                self.x -= rhs.x; self.y -= rhs.y; self.z -= rhs.z;
            }
        }

        impl core::ops::Mul for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: Self) -> Self { Self::new(self.x * rhs.x, self.y * rhs.y, self.z * rhs.z) }
        }

        impl core::ops::MulAssign for Vector3T<$t> {
            #[inline]
            fn mul_assign(&mut self, rhs: Self) {
                self.x *= rhs.x; self.y *= rhs.y; self.z *= rhs.z;
            }
        }

        impl core::ops::Div for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn div(self, rhs: Self) -> Self { Self::new(self.x / rhs.x, self.y / rhs.y, self.z / rhs.z) }
        }

        impl core::ops::DivAssign for Vector3T<$t> {
            #[inline]
            fn div_assign(&mut self, rhs: Self) {
                self.x /= rhs.x; self.y /= rhs.y; self.z /= rhs.z;
            }
        }

        impl core::ops::Mul<$t> for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, s: $t) -> Self { Self::new(self.x * s, self.y * s, self.z * s) }
        }

        impl core::ops::MulAssign<$t> for Vector3T<$t> {
            #[inline]
            fn mul_assign(&mut self, s: $t) {
                self.x *= s; self.y *= s; self.z *= s;
            }
        }

        // scalar * vector (commutative, matches the free operator* in vector3t.h)
        impl core::ops::Mul<Vector3T<$t>> for $t {
            type Output = Vector3T<$t>;
            #[inline]
            fn mul(self, v: Vector3T<$t>) -> Vector3T<$t> { v * self }
        }

        impl core::ops::Div<$t> for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn div(self, s: $t) -> Self { Self::new(self.x / s, self.y / s, self.z / s) }
        }

        impl core::ops::DivAssign<$t> for Vector3T<$t> {
            #[inline]
            fn div_assign(&mut self, s: $t) {
                self.x /= s; self.y /= s; self.z /= s;
            }
        }

        impl core::ops::Neg for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self { Self::new(-self.x, -self.y, -self.z) }
        }
    };
}

impl_vector_ops!(f32, 0.0);
impl_vector_ops!(f64, 0.0);
impl_vector_ops!(i32, 0);
impl_vector_ops!(i64, 0);

// ---- Ordering (matches operator< in vector3t.h, used for sorted containers) ----

impl<T: Copy + PartialOrd> PartialOrd for Vector3T<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp_lex(other))
    }
}

impl<T: Copy + PartialOrd> Vector3T<T> {
    /// Lexicographic ordering matching C++ `operator<` (x → y → z).
    pub fn cmp_lex(&self, o: &Self) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        if self.x.partial_cmp(&o.x) != Some(Ordering::Equal) {
            return self.x.partial_cmp(&o.x).unwrap_or(Ordering::Equal);
        }
        if self.y.partial_cmp(&o.y) != Some(Ordering::Equal) {
            return self.y.partial_cmp(&o.y).unwrap_or(Ordering::Equal);
        }
        self.z.partial_cmp(&o.z).unwrap_or(Ordering::Equal)
    }
}

// ---- Free math functions on Vector3f (ported from vector3f.h / vector3t.h math::) ----

pub type Vector3f = Vector3T<f32>;
pub type Vector3d = Vector3T<f64>;
pub type Vector3i = Vector3T<i32>;

// Vector3f-specific math namespace (mirrors `zylann::math::` overloads for Vector3f)
pub mod math {
    use super::*;

    #[inline]
    pub fn floor(v: Vector3f) -> Vector3f {
        Vector3f::new(v.x.floor(), v.y.floor(), v.z.floor())
    }

    #[inline]
    pub fn ceil(v: Vector3f) -> Vector3f {
        Vector3f::new(v.x.ceil(), v.y.ceil(), v.z.ceil())
    }

    #[inline]
    pub fn lerp(a: Vector3f, b: Vector3f, t: f32) -> Vector3f {
        Vector3f::new(
            funcs::lerp_f32(a.x, b.x, t),
            funcs::lerp_f32(a.y, b.y, t),
            funcs::lerp_f32(a.z, b.z, t),
        )
    }

    #[inline]
    pub fn has_nan(v: Vector3f) -> bool {
        v.x.is_nan() || v.y.is_nan() || v.z.is_nan()
    }

    #[inline]
    pub fn length_squared(v: Vector3f) -> f32 {
        v.x * v.x + v.y * v.y + v.z * v.z
    }

    #[inline]
    pub fn length(v: Vector3f) -> f32 {
        funcs::sqrt_f32(length_squared(v))
    }

    #[inline]
    pub fn distance_squared(a: Vector3f, b: Vector3f) -> f32 {
        length_squared(b - a)
    }

    #[inline]
    pub fn distance(a: Vector3f, b: Vector3f) -> f32 {
        funcs::sqrt_f32(distance_squared(a, b))
    }

    #[inline]
    pub fn cross(a: Vector3f, b: Vector3f) -> Vector3f {
        Vector3f::new(
            a.y * b.z - a.z * b.y,
            a.z * b.x - a.x * b.z,
            a.x * b.y - a.y * b.x,
        )
    }

    #[inline]
    pub fn dot(a: Vector3f, b: Vector3f) -> f32 {
        a.x * b.x + a.y * b.y + a.z * b.z
    }

    #[inline]
    pub fn abs(v: Vector3f) -> Vector3f {
        Vector3f::new(v.x.abs(), v.y.abs(), v.z.abs())
    }

    pub fn normalized(v: Vector3f) -> Vector3f {
        let lsq = length_squared(v);
        if lsq == 0.0 {
            Vector3f::zero()
        } else {
            v / funcs::sqrt_f32(lsq)
        }
    }

    /// Returns unit vector and writes the original length into `out_length`.
    pub fn normalized_with_length(v: Vector3f, out_length: &mut f32) -> Vector3f {
        let lsq = length_squared(v);
        if lsq == 0.0 {
            *out_length = 0.0;
            Vector3f::zero()
        } else {
            let l = funcs::sqrt_f32(lsq);
            *out_length = l;
            v / l
        }
    }

    pub fn is_normalized(v: Vector3f) -> bool {
        // Use length_squared() to avoid sqrt — more stringent (matches C++).
        funcs::is_equal_approx(length_squared(v), 1.0)
    }

    pub fn is_equal_approx(a: Vector3f, b: Vector3f) -> bool {
        funcs::is_equal_approx(a.x, b.x)
            && funcs::is_equal_approx(a.y, b.y)
            && funcs::is_equal_approx(a.z, b.z)
    }

    pub fn min(a: Vector3f, b: Vector3f) -> Vector3f {
        Vector3f::new(funcs::min(a.x, b.x), funcs::min(a.y, b.y), funcs::min(a.z, b.z))
    }

    pub fn max(a: Vector3f, b: Vector3f) -> Vector3f {
        Vector3f::new(funcs::max(a.x, b.x), funcs::max(a.y, b.y), funcs::max(a.z, b.z))
    }

    pub fn clamp(v: Vector3f, lo: Vector3f, hi: Vector3f) -> Vector3f {
        Vector3f::new(
            funcs::clamp(v.x, lo.x, hi.x),
            funcs::clamp(v.y, lo.y, hi.y),
            funcs::clamp(v.z, lo.z, hi.z),
        )
    }

    pub fn get_longest_axis(v: Vector3f) -> Axis {
        let a = abs(v);
        if a.x > a.y {
            if a.x > a.z { Axis::X } else { Axis::Z }
        } else if a.y > a.z {
            Axis::Y
        } else {
            Axis::Z
        }
    }

    // 90° rotations (match vector3t.h). CCW = positive angle (axis pointing at viewer).
    #[inline] pub fn rotate_x_90_ccw(v: Vector3f) -> Vector3f { Vector3f::new(v.x, -v.z, v.y) }
    #[inline] pub fn rotate_x_90_cw(v: Vector3f)   -> Vector3f { Vector3f::new(v.x,  v.z, -v.y) }
    #[inline] pub fn rotate_y_90_ccw(v: Vector3f) -> Vector3f { Vector3f::new(v.z, v.y, -v.x) }
    #[inline] pub fn rotate_y_90_cw(v: Vector3f)   -> Vector3f { Vector3f::new(-v.z, v.y, v.x) }
    #[inline] pub fn rotate_z_90_ccw(v: Vector3f) -> Vector3f { Vector3f::new(-v.y, v.x, v.z) }
    #[inline] pub fn rotate_z_90_cw(v: Vector3f)   -> Vector3f { Vector3f::new(v.y, -v.x, v.z) }

    pub fn is_valid_size(s: Vector3f) -> bool {
        s.x >= 0.0 && s.y >= 0.0 && s.z >= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ops_f32() {
        let a = Vector3f::new(1.0, 2.0, 3.0);
        let b = Vector3f::new(4.0, 5.0, 6.0);
        assert_eq!(a + b, Vector3f::new(5.0, 7.0, 9.0));
        assert_eq!(b - a, Vector3f::new(3.0, 3.0, 3.0));
        assert_eq!(a * 2.0, Vector3f::new(2.0, 4.0, 6.0));
        assert_eq!(2.0 * a, Vector3f::new(2.0, 4.0, 6.0));
        assert_eq!(-a, Vector3f::new(-1.0, -2.0, -3.0));
    }

    #[test]
    fn dot_and_cross() {
        let x = Vector3f::new(1.0, 0.0, 0.0);
        let y = Vector3f::new(0.0, 1.0, 0.0);
        assert_eq!(math::dot(x, y), 0.0);
        assert_eq!(math::cross(x, y), Vector3f::new(0.0, 0.0, 1.0));
        assert_eq!(math::length(x), 1.0);
    }

    #[test]
    fn normalize() {
        let v = Vector3f::new(0.0, 0.0, 0.0);
        assert_eq!(math::normalized(v), Vector3f::zero());
        assert!(math::is_normalized(math::normalized(Vector3f::new(3.0, 0.0, 0.0))));
    }

    #[test]
    fn swizzles() {
        let v = Vector3f::new(1.0, 2.0, 3.0);
        assert_eq!(v.zyx(), Vector3f::new(3.0, 2.0, 1.0));
        assert_eq!(v.yzx(), Vector3f::new(2.0, 3.0, 1.0));
        assert_eq!(v.zxy(), Vector3f::new(3.0, 1.0, 2.0));
    }

    #[test]
    fn rotations() {
        let x = Vector3f::new(1.0, 0.0, 0.0);
        // Rotating X axis around Y by 90° CW sends +X to -Z
        assert_eq!(math::rotate_y_90_cw(x), Vector3f::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn longest_axis() {
        assert_eq!(math::get_longest_axis(Vector3f::new(1.0, 2.0, 3.0)), Axis::Z);
        assert_eq!(math::get_longest_axis(Vector3f::new(5.0, 2.0, 3.0)), Axis::X);
        assert_eq!(math::get_longest_axis(Vector3f::new(1.0, 9.0, 3.0)), Axis::Y);
    }

    #[test]
    fn lex_ordering() {
        let a = Vector3f::new(1.0, 2.0, 3.0);
        let b = Vector3f::new(1.0, 2.0, 4.0);
        assert!(a < b);
        assert!(a <= a);
    }

    #[test]
    fn vector3i_ops() {
        let a = Vector3i::new(1, 2, 3);
        let b = Vector3i::new(4, 5, 6);
        assert_eq!(a + b, Vector3i::new(5, 7, 9));
    }
}
