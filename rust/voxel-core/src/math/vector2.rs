//! 2D vector types.
//!
//! Ported from `util/math/vector2t.h` (template `Vector2T<T>`), `vector2f.h`
//! (free math on `Vector2f = Vector2T<f32>`) and `vector2i.h` (`Vector2iUtil`
//! helpers + integer math). Mirrors the [`super::vector3`] module layout.

use super::funcs;

/// Generic 2D vector. Matches `Vector2T<T>` in C++. `#[repr(C)]` so it matches
/// the C++ `union { struct {x,y}; T coords[2]; }` layout for FFI / GPU upload.
#[derive(Debug, Clone, Copy, PartialEq, Default, Hash)]
#[repr(C)]
pub struct Vector2T<T: Copy> {
    pub x: T,
    pub y: T,
}

impl<T: Copy> Vector2T<T> {
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    /// Broadcast a scalar to both axes (matches the `explicit Vector2T(T)` ctor;
    /// kept named to avoid implicit conversions).
    pub const fn splat(v: T) -> Self {
        Self { x: v, y: v }
    }

    #[inline]
    pub fn axis(&self, axis: usize) -> T {
        match axis {
            0 => self.x,
            1 => self.y,
            _ => panic!("Vector2 axis out of range"),
        }
    }
}

impl<T: Copy + PartialOrd> Vector2T<T> {
    /// Lexicographic ordering x → y (Rust idiom; C++ Vector2T has no operator<).
    pub fn cmp_lex(&self, o: &Self) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        match self.x.partial_cmp(&o.x) {
            Some(Ordering::Equal) | None => {}
            Some(ord) => return ord,
        }
        self.y.partial_cmp(&o.y).unwrap_or(Ordering::Equal)
    }
}

impl<T: Copy + PartialOrd> PartialOrd for Vector2T<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp_lex(other))
    }
}

impl<T: Copy> core::ops::Index<usize> for Vector2T<T> {
    type Output = T;
    #[inline]
    fn index(&self, axis: usize) -> &T {
        match axis {
            0 => &self.x,
            1 => &self.y,
            _ => panic!("Vector2 axis out of range"),
        }
    }
}

impl<T: Copy> core::ops::IndexMut<usize> for Vector2T<T> {
    #[inline]
    fn index_mut(&mut self, axis: usize) -> &mut T {
        match axis {
            0 => &mut self.x,
            1 => &mut self.y,
            _ => panic!("Vector2 axis out of range"),
        }
    }
}

// ---- Operator overloading via std::ops (concrete numeric cases) ----

macro_rules! impl_vector2_ops {
    ($t:ty, $zero:expr) => {
        impl Vector2T<$t> {
            #[inline]
            pub fn zero() -> Self {
                Self::splat($zero)
            }
        }

        impl core::ops::Add for Vector2T<$t> {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self {
                Self::new(self.x + rhs.x, self.y + rhs.y)
            }
        }
        impl core::ops::AddAssign for Vector2T<$t> {
            #[inline]
            fn add_assign(&mut self, rhs: Self) {
                self.x += rhs.x;
                self.y += rhs.y;
            }
        }
        impl core::ops::Sub for Vector2T<$t> {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: Self) -> Self {
                Self::new(self.x - rhs.x, self.y - rhs.y)
            }
        }
        impl core::ops::SubAssign for Vector2T<$t> {
            #[inline]
            fn sub_assign(&mut self, rhs: Self) {
                self.x -= rhs.x;
                self.y -= rhs.y;
            }
        }
        impl core::ops::Mul for Vector2T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: Self) -> Self {
                Self::new(self.x * rhs.x, self.y * rhs.y)
            }
        }
        impl core::ops::MulAssign for Vector2T<$t> {
            #[inline]
            fn mul_assign(&mut self, rhs: Self) {
                self.x *= rhs.x;
                self.y *= rhs.y;
            }
        }
        impl core::ops::Div for Vector2T<$t> {
            type Output = Self;
            #[inline]
            fn div(self, rhs: Self) -> Self {
                Self::new(self.x / rhs.x, self.y / rhs.y)
            }
        }
        impl core::ops::DivAssign for Vector2T<$t> {
            #[inline]
            fn div_assign(&mut self, rhs: Self) {
                self.x /= rhs.x;
                self.y /= rhs.y;
            }
        }
        impl core::ops::Mul<$t> for Vector2T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, s: $t) -> Self {
                Self::new(self.x * s, self.y * s)
            }
        }
        impl core::ops::MulAssign<$t> for Vector2T<$t> {
            #[inline]
            fn mul_assign(&mut self, s: $t) {
                self.x *= s;
                self.y *= s;
            }
        }
        // scalar * vector (commutative, matches the free operator* in vector2t.h)
        impl core::ops::Mul<Vector2T<$t>> for $t {
            type Output = Vector2T<$t>;
            #[inline]
            fn mul(self, v: Vector2T<$t>) -> Vector2T<$t> {
                v * self
            }
        }
        impl core::ops::Div<$t> for Vector2T<$t> {
            type Output = Self;
            #[inline]
            fn div(self, s: $t) -> Self {
                Self::new(self.x / s, self.y / s)
            }
        }
        impl core::ops::DivAssign<$t> for Vector2T<$t> {
            #[inline]
            fn div_assign(&mut self, s: $t) {
                self.x /= s;
                self.y /= s;
            }
        }
        impl core::ops::Neg for Vector2T<$t> {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                Self::new(-self.x, -self.y)
            }
        }
    };
}

impl_vector2_ops!(f32, 0.0);
impl_vector2_ops!(f64, 0.0);
impl_vector2_ops!(i32, 0);
impl_vector2_ops!(i64, 0);

// ---- Type aliases ----

pub type Vector2f = Vector2T<f32>;
pub type Vector2d = Vector2T<f64>;
pub type Vector2i = Vector2T<i32>;

// ---- Integer helpers (ported from vector2i.h Vector2iUtil / math::) ----

impl Vector2i {
    /// `x * y` as i64 (matches `Vector2iUtil::get_area`).
    #[inline]
    pub fn area(self) -> i64 {
        debug_assert!(self.x >= 0 && self.y >= 0);
        self.x as i64 * self.y as i64
    }

    /// Row-major (YX) flat index into a buffer of size `area_size`.
    /// Matches `Vector2iUtil::get_yx_index`: `x + y * area_size.x`.
    #[inline]
    pub fn yx_index(self, area_size: Vector2i) -> usize {
        self.x as usize + self.y as usize * area_size.x as usize
    }

    /// Component-wise floor division. Matches `math::floordiv(Vector2i, Vector2i)`.
    #[inline]
    pub fn floordiv(self, d: Vector2i) -> Vector2i {
        Vector2i::new(funcs::floordiv(self.x, d.x), funcs::floordiv(self.y, d.y))
    }

    /// Floor division by a scalar. Matches `math::floordiv(Vector2i, int)`.
    #[inline]
    pub fn floordiv_scalar(self, d: i32) -> Vector2i {
        Vector2i::new(funcs::floordiv(self.x, d), funcs::floordiv(self.y, d))
    }

    /// Component-wise ceil division. Matches `math::ceildiv(Vector2i, Vector2i)`.
    #[inline]
    pub fn ceildiv(self, d: Vector2i) -> Vector2i {
        Vector2i::new(funcs::ceildiv(self.x, d.x), funcs::ceildiv(self.y, d.y))
    }

    /// Ceil division by a scalar. Matches `math::ceildiv(Vector2i, int)`.
    #[inline]
    pub fn ceildiv_scalar(self, d: i32) -> Vector2i {
        Vector2i::new(funcs::ceildiv(self.x, d), funcs::ceildiv(self.y, d))
    }

    /// Chebyshev (chessboard) distance. Matches `math::chebyshev_distance`.
    #[inline]
    pub fn chebyshev_distance_to(self, b: Vector2i) -> i32 {
        funcs::max(funcs::abs_i32(self.x - b.x), funcs::abs_i32(self.y - b.y))
    }
}

// ---- Free math functions on Vector2f (ported from vector2f.h / vector2t.h math::) ----

/// `Vector2f` math, mirroring `zylann::math::` overloads.
pub mod math {
    use super::*;

    #[inline]
    pub fn floor(v: Vector2f) -> Vector2f {
        Vector2f::new(v.x.floor(), v.y.floor())
    }

    #[inline]
    pub fn ceil(v: Vector2f) -> Vector2f {
        Vector2f::new(v.x.ceil(), v.y.ceil())
    }

    #[inline]
    pub fn lerp(a: Vector2f, b: Vector2f, t: f32) -> Vector2f {
        Vector2f::new(funcs::lerp_f32(a.x, b.x, t), funcs::lerp_f32(a.y, b.y, t))
    }

    #[inline]
    pub fn is_equal_approx(a: Vector2f, b: Vector2f) -> bool {
        funcs::is_equal_approx(a.x, b.x) && funcs::is_equal_approx(a.y, b.y)
    }

    #[inline]
    pub fn abs(v: Vector2f) -> Vector2f {
        Vector2f::new(funcs::abs_f32(v.x), funcs::abs_f32(v.y))
    }

    #[inline]
    pub fn sign_nonzero(v: Vector2f) -> Vector2f {
        Vector2f::new(funcs::sign_nonzero_f32(v.x), funcs::sign_nonzero_f32(v.y))
    }

    #[inline]
    pub fn dot(a: Vector2f, b: Vector2f) -> f32 {
        a.x * b.x + a.y * b.y
    }

    /// 2D scalar cross product `a.x*b.y - a.y*b.x` (signed area).
    #[inline]
    pub fn cross(a: Vector2f, b: Vector2f) -> f32 {
        a.x * b.y - a.y * b.x
    }

    #[inline]
    pub fn length_squared(v: Vector2f) -> f32 {
        v.x * v.x + v.y * v.y
    }

    #[inline]
    pub fn length(v: Vector2f) -> f32 {
        funcs::sqrt_f32(length_squared(v))
    }

    #[inline]
    pub fn distance_squared(a: Vector2f, b: Vector2f) -> f32 {
        length_squared(b - a)
    }

    #[inline]
    pub fn distance(a: Vector2f, b: Vector2f) -> f32 {
        funcs::sqrt_f32(distance_squared(a, b))
    }

    #[inline]
    pub fn min(a: Vector2f, b: Vector2f) -> Vector2f {
        Vector2f::new(funcs::min(a.x, b.x), funcs::min(a.y, b.y))
    }

    #[inline]
    pub fn max(a: Vector2f, b: Vector2f) -> Vector2f {
        Vector2f::new(funcs::max(a.x, b.x), funcs::max(a.y, b.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_f32() {
        let a = Vector2f::new(1.0, 2.0);
        let b = Vector2f::new(3.0, 4.0);
        assert_eq!(a + b, Vector2f::new(4.0, 6.0));
        assert_eq!(b - a, Vector2f::new(2.0, 2.0));
        assert_eq!(a * 2.0, Vector2f::new(2.0, 4.0));
        assert_eq!(2.0 * a, Vector2f::new(2.0, 4.0));
        assert_eq!(-a, Vector2f::new(-1.0, -2.0));
    }

    #[test]
    fn ops_i32() {
        let a = Vector2i::new(1, 2);
        let b = Vector2i::new(3, 4);
        assert_eq!(a + b, Vector2i::new(4, 6));
        assert_eq!(a * 3, Vector2i::new(3, 6));
        assert_eq!(Vector2i::splat(5) * a, Vector2i::new(5, 10));
    }

    #[test]
    fn index_access() {
        let mut v = Vector2f::new(10.0, 20.0);
        assert_eq!(v[0], 10.0);
        assert_eq!(v[1], 20.0);
        v[1] = 99.0;
        assert_eq!(v.y, 99.0);
    }

    #[test]
    fn lex_ordering() {
        let a = Vector2i::new(1, 5);
        let b = Vector2i::new(2, 0);
        assert!(a < b); // x dominates
        let c = Vector2i::new(1, 5);
        let d = Vector2i::new(1, 6);
        assert!(c < d); // y breaks tie
    }

    #[test]
    fn dot_cross_length() {
        let a = Vector2f::new(3.0, 0.0);
        let b = Vector2f::new(0.0, 4.0);
        assert_eq!(math::dot(a, b), 0.0);
        assert_eq!(math::cross(a, b), 12.0);
        assert_eq!(math::length(a), 3.0);
        assert_eq!(math::length(b), 4.0);
        assert_eq!(math::distance(a, b), 5.0);
    }

    #[test]
    fn lerp_floor_is_equal() {
        assert_eq!(
            math::lerp(Vector2f::new(0.0, 0.0), Vector2f::new(10.0, 20.0), 0.5),
            Vector2f::new(5.0, 10.0)
        );
        assert_eq!(
            math::floor(Vector2f::new(-0.5, 1.9)),
            Vector2f::new(-1.0, 1.0)
        );
        assert!(math::is_equal_approx(
            Vector2f::new(1.0, 1.0),
            Vector2f::new(1.0, 1.0)
        ));
    }

    #[test]
    fn vector2i_helpers() {
        let v = Vector2i::new(3, 4);
        assert_eq!(v.area(), 12);
        assert_eq!(Vector2i::new(2, 1).yx_index(Vector2i::new(10, 10)), 12); // 2 + 1*10
        assert_eq!(
            Vector2i::new(-1, 7).floordiv_scalar(3),
            Vector2i::new(-1, 2)
        );
        assert_eq!(
            Vector2i::new(0, 0).chebyshev_distance_to(Vector2i::new(3, 4)),
            4
        );
    }

    #[test]
    fn splat_and_zero() {
        assert_eq!(Vector2i::splat(7), Vector2i::new(7, 7));
        assert_eq!(Vector2f::zero(), Vector2f::new(0.0, 0.0));
    }
}
