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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

    /// Index access with runtime bounds check. Mirrors `operator[]`.
    #[inline]
    pub fn get(&self, i: usize) -> T {
        match i {
            0 => self.x,
            1 => self.y,
            2 => self.z,
            _ => panic!("Vector3 index out of range"),
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: T) {
        match i {
            0 => self.x = v,
            1 => self.y = v,
            2 => self.z = v,
            _ => panic!("Vector3 index out of range"),
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
    pub fn xyz(self) -> Self {
        self
    }

    #[inline]
    pub fn zyx(self) -> Self {
        Self {
            x: self.z,
            y: self.y,
            z: self.x,
        }
    }

    #[inline]
    pub fn zxy(self) -> Self {
        Self {
            x: self.z,
            y: self.x,
            z: self.y,
        }
    }

    #[inline]
    pub fn yzx(self) -> Self {
        Self {
            x: self.y,
            y: self.z,
            z: self.x,
        }
    }
}

// ---- Operator overloading via std::ops ----
// We impl for the concrete numeric cases we actually need rather than
// trying to be fully generic; that keeps the trait bounds tractable and
// matches how the C++ template is instantiated (Vector3f / Vector3i).

macro_rules! impl_vector_ops {
    ($t:ty, $zero:expr) => {
        impl Vector3T<$t> {
            #[inline]
            pub fn zero() -> Self {
                Self::splat($zero)
            }
        }

        impl core::ops::Add for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self {
                Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
            }
        }

        impl core::ops::AddAssign for Vector3T<$t> {
            #[inline]
            fn add_assign(&mut self, rhs: Self) {
                self.x += rhs.x;
                self.y += rhs.y;
                self.z += rhs.z;
            }
        }

        impl core::ops::Sub for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: Self) -> Self {
                Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
            }
        }

        impl core::ops::SubAssign for Vector3T<$t> {
            #[inline]
            fn sub_assign(&mut self, rhs: Self) {
                self.x -= rhs.x;
                self.y -= rhs.y;
                self.z -= rhs.z;
            }
        }

        impl core::ops::Mul for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: Self) -> Self {
                Self::new(self.x * rhs.x, self.y * rhs.y, self.z * rhs.z)
            }
        }

        impl core::ops::MulAssign for Vector3T<$t> {
            #[inline]
            fn mul_assign(&mut self, rhs: Self) {
                self.x *= rhs.x;
                self.y *= rhs.y;
                self.z *= rhs.z;
            }
        }

        impl core::ops::Div for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn div(self, rhs: Self) -> Self {
                Self::new(self.x / rhs.x, self.y / rhs.y, self.z / rhs.z)
            }
        }

        impl core::ops::DivAssign for Vector3T<$t> {
            #[inline]
            fn div_assign(&mut self, rhs: Self) {
                self.x /= rhs.x;
                self.y /= rhs.y;
                self.z /= rhs.z;
            }
        }

        impl core::ops::Mul<$t> for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, s: $t) -> Self {
                Self::new(self.x * s, self.y * s, self.z * s)
            }
        }

        impl core::ops::MulAssign<$t> for Vector3T<$t> {
            #[inline]
            fn mul_assign(&mut self, s: $t) {
                self.x *= s;
                self.y *= s;
                self.z *= s;
            }
        }

        // scalar * vector (commutative, matches the free operator* in vector3t.h)
        impl core::ops::Mul<Vector3T<$t>> for $t {
            type Output = Vector3T<$t>;
            #[inline]
            fn mul(self, v: Vector3T<$t>) -> Vector3T<$t> {
                v * self
            }
        }

        impl core::ops::Div<$t> for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn div(self, s: $t) -> Self {
                Self::new(self.x / s, self.y / s, self.z / s)
            }
        }

        impl core::ops::DivAssign<$t> for Vector3T<$t> {
            #[inline]
            fn div_assign(&mut self, s: $t) {
                self.x /= s;
                self.y /= s;
                self.z /= s;
            }
        }

        impl core::ops::Neg for Vector3T<$t> {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                Self::new(-self.x, -self.y, -self.z)
            }
        }
    };
}

impl_vector_ops!(f32, 0.0);
impl_vector_ops!(f64, 0.0);
impl_vector_ops!(i16, 0);
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
pub type Vector3i16 = Vector3T<i16>;

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
        Vector3f::new(
            funcs::min(a.x, b.x),
            funcs::min(a.y, b.y),
            funcs::min(a.z, b.z),
        )
    }

    pub fn max(a: Vector3f, b: Vector3f) -> Vector3f {
        Vector3f::new(
            funcs::max(a.x, b.x),
            funcs::max(a.y, b.y),
            funcs::max(a.z, b.z),
        )
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
            if a.x > a.z {
                Axis::X
            } else {
                Axis::Z
            }
        } else if a.y > a.z {
            Axis::Y
        } else {
            Axis::Z
        }
    }

    // 90° rotations (match vector3t.h). CCW = positive angle (axis pointing at viewer).
    #[inline]
    pub fn rotate_x_90_ccw(v: Vector3f) -> Vector3f {
        Vector3f::new(v.x, -v.z, v.y)
    }
    #[inline]
    pub fn rotate_x_90_cw(v: Vector3f) -> Vector3f {
        Vector3f::new(v.x, v.z, -v.y)
    }
    #[inline]
    pub fn rotate_y_90_ccw(v: Vector3f) -> Vector3f {
        Vector3f::new(v.z, v.y, -v.x)
    }
    #[inline]
    pub fn rotate_y_90_cw(v: Vector3f) -> Vector3f {
        Vector3f::new(-v.z, v.y, v.x)
    }
    #[inline]
    pub fn rotate_z_90_ccw(v: Vector3f) -> Vector3f {
        Vector3f::new(-v.y, v.x, v.z)
    }
    #[inline]
    pub fn rotate_z_90_cw(v: Vector3f) -> Vector3f {
        Vector3f::new(v.y, -v.x, v.z)
    }

    pub fn is_valid_size(s: Vector3f) -> bool {
        s.x >= 0.0 && s.y >= 0.0 && s.z >= 0.0
    }
}

// ---- Vector3i utilities (ported from vector3i.h Vector3iUtil / math::) ----

impl Vector3i {
    /// Broadcast a scalar to all axes. Matches `Vector3iUtil::create(int)`.
    #[inline]
    pub fn create(xyz: i32) -> Self {
        Self::splat(xyz)
    }

    /// Sort two vectors component-wise into (min, max). Matches
    /// `Vector3iUtil::sort_min_max`.
    #[inline]
    pub fn sort_min_max(a: &mut Vector3i, b: &mut Vector3i) {
        funcs::sort2(&mut a.x, &mut b.x);
        funcs::sort2(&mut a.y, &mut b.y);
        funcs::sort2(&mut a.z, &mut b.z);
    }

    /// Unsigned volume `x*y*z` as `u64` (overflows checked in debug).
    /// Matches `Vector3iUtil::get_volume_u64`. Expects non-negative components.
    #[inline]
    pub fn volume_u64(self) -> u64 {
        debug_assert!(self.x >= 0 && self.y >= 0 && self.z >= 0);
        funcs::multiply_check_overflow_u64(
            self.x as u64,
            funcs::multiply_check_overflow_u64(self.y as u64, self.z as u64),
        )
    }

    /// Row-major ZXY flat index into a buffer of size `area_size`.
    /// Matches `Vector3iUtil::get_zxy_index`: `y + sy*(x + sx*z)`.
    #[inline]
    pub fn zxy_index(self, area_size: Vector3i) -> u32 {
        (self.y as u32).wrapping_add((area_size.y as u32).wrapping_mul(
            (self.x as u32).wrapping_add((area_size.x as u32).wrapping_mul(self.z as u32)),
        ))
    }

    /// Same as [`zxy_index`](Self::zxy_index) taking scalars. Matches the
    /// 5-argument overload `get_zxy_index(x, y, z, sx, sy)`.
    #[inline]
    pub fn zxy_index_scalars(x: i32, y: i32, z: i32, sx: i32, sy: i32) -> u32 {
        (y as u32).wrapping_add(
            (sy as u32).wrapping_mul((x as u32).wrapping_add((sx as u32).wrapping_mul(z as u32))),
        )
    }

    /// Row-major ZYX flat index. Matches `Vector3iUtil::get_zyx_index`:
    /// `x + sx*(y + sy*z)`.
    #[inline]
    pub fn zyx_index(self, area_size: Vector3i) -> u32 {
        (self.x as u32).wrapping_add((area_size.x as u32).wrapping_mul(
            (self.y as u32).wrapping_add((area_size.y as u32).wrapping_mul(self.z as u32)),
        ))
    }

    /// Inverse of [`zxy_index`](Self::zxy_index). Matches `from_zxy_index`.
    #[inline]
    pub fn from_zxy_index(i: u32, area_size: Vector3i) -> Vector3i {
        Vector3i::new(
            (i / area_size.y as u32) as i32 % area_size.x,
            (i % area_size.y as u32) as i32,
            (i / (area_size.y as u32 * area_size.x as u32)) as i32,
        )
    }

    /// True if all three components are equal. Matches `all_members_equal`.
    #[inline]
    pub fn all_members_equal(self) -> bool {
        self.x == self.y && self.y == self.z
    }

    /// True if the vector has length 1 (sum of abs components == 1).
    /// Matches `Vector3iUtil::is_unit_vector`.
    #[inline]
    pub fn is_unit_vector(self) -> bool {
        funcs::abs_i32(self.x) + funcs::abs_i32(self.y) + funcs::abs_i32(self.z) == 1
    }

    /// True if all components are non-negative. Matches `is_valid_size`.
    #[inline]
    pub fn is_valid_size(self) -> bool {
        self.x >= 0 && self.y >= 0 && self.z >= 0
    }

    /// True if any component is zero. Matches `is_empty_size`.
    #[inline]
    pub fn is_empty_size(self) -> bool {
        self.x == 0 || self.y == 0 || self.z == 0
    }

    /// Manhattan (L1) distance. Matches `math::manhattan_distance`.
    #[inline]
    pub fn manhattan_distance_to(self, b: Vector3i) -> i32 {
        funcs::abs_i32(self.x - b.x) + funcs::abs_i32(self.y - b.y) + funcs::abs_i32(self.z - b.z)
    }

    /// Chebyshev (chessboard) distance. Matches `math::chebyshev_distance`.
    #[inline]
    pub fn chebyshev_distance_to(self, b: Vector3i) -> i32 {
        funcs::max(
            funcs::max(funcs::abs_i32(self.x - b.x), funcs::abs_i32(self.y - b.y)),
            funcs::abs_i32(self.z - b.z),
        )
    }

    /// Integer dot product. Matches `math::dot(Vector3i, Vector3i)`.
    #[inline]
    pub fn dot(self, b: Vector3i) -> i32 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }

    /// Component-wise abs. Matches `math::abs(Vector3i)`.
    #[inline]
    pub fn abs(self) -> Vector3i {
        Vector3i::new(
            funcs::abs_i32(self.x),
            funcs::abs_i32(self.y),
            funcs::abs_i32(self.z),
        )
    }

    /// Component-wise min. Matches `math::min(Vector3i, Vector3i)`.
    #[inline]
    pub fn min(self, b: Vector3i) -> Vector3i {
        Vector3i::new(
            funcs::min(self.x, b.x),
            funcs::min(self.y, b.y),
            funcs::min(self.z, b.z),
        )
    }

    /// Component-wise max. Matches `math::max(Vector3i, Vector3i)`.
    #[inline]
    pub fn max(self, b: Vector3i) -> Vector3i {
        Vector3i::new(
            funcs::max(self.x, b.x),
            funcs::max(self.y, b.y),
            funcs::max(self.z, b.z),
        )
    }

    /// Component-wise clamp. Matches `math::clamp(Vector3i, ...)`.
    #[inline]
    pub fn clamp(self, lo: Vector3i, hi: Vector3i) -> Vector3i {
        Vector3i::new(
            funcs::clamp(self.x, lo.x, hi.x),
            funcs::clamp(self.y, lo.y, hi.y),
            funcs::clamp(self.z, lo.z, hi.z),
        )
    }

    /// Component-wise floor division by another vector.
    /// Matches `math::floordiv(Vector3i, Vector3i)`.
    #[inline]
    pub fn floordiv(self, d: Vector3i) -> Vector3i {
        Vector3i::new(
            funcs::floordiv(self.x, d.x),
            funcs::floordiv(self.y, d.y),
            funcs::floordiv(self.z, d.z),
        )
    }

    /// Floor division by a scalar. Matches `math::floordiv(Vector3i, int)`.
    #[inline]
    pub fn floordiv_scalar(self, d: i32) -> Vector3i {
        Vector3i::new(
            funcs::floordiv(self.x, d),
            funcs::floordiv(self.y, d),
            funcs::floordiv(self.z, d),
        )
    }

    /// Component-wise ceil division. Matches `math::ceildiv(Vector3i, Vector3i)`.
    #[inline]
    pub fn ceildiv(self, d: Vector3i) -> Vector3i {
        Vector3i::new(
            funcs::ceildiv(self.x, d.x),
            funcs::ceildiv(self.y, d.y),
            funcs::ceildiv(self.z, d.z),
        )
    }

    /// Ceil division by a scalar. Matches `math::ceildiv(Vector3i, int)`.
    #[inline]
    pub fn ceildiv_scalar(self, d: i32) -> Vector3i {
        Vector3i::new(
            funcs::ceildiv(self.x, d),
            funcs::ceildiv(self.y, d),
            funcs::ceildiv(self.z, d),
        )
    }

    /// Component-wise wrap. Matches `math::wrap(Vector3i, Vector3i)`.
    #[inline]
    pub fn wrap(self, d: Vector3i) -> Vector3i {
        Vector3i::new(
            funcs::wrap_i32(self.x, d.x),
            funcs::wrap_i32(self.y, d.y),
            funcs::wrap_i32(self.z, d.z),
        )
    }
}

// ---- 90° rotations on Vector3i (match vector3i.h math::rotate_*_90_*) ----
// CW/CW convention: axis pointed at the viewer; CCW = positive angle.
impl Vector3i {
    #[inline]
    pub fn rotate_x_90_ccw(self) -> Vector3i {
        Vector3i::new(self.x, -self.z, self.y)
    }
    #[inline]
    pub fn rotate_x_90_cw(self) -> Vector3i {
        Vector3i::new(self.x, self.z, -self.y)
    }
    #[inline]
    pub fn rotate_y_90_ccw(self) -> Vector3i {
        Vector3i::new(self.z, self.y, -self.x)
    }
    #[inline]
    pub fn rotate_y_90_cw(self) -> Vector3i {
        Vector3i::new(-self.z, self.y, self.x)
    }
    #[inline]
    pub fn rotate_z_90_ccw(self) -> Vector3i {
        Vector3i::new(-self.y, self.x, self.z)
    }
    #[inline]
    pub fn rotate_z_90_cw(self) -> Vector3i {
        Vector3i::new(self.y, -self.x, self.z)
    }

    /// Dispatch 90° rotation by [`Axis`]. Matches `math::rotate_90(Vector3i, ...)`.
    /// `clockwise = true` picks the CW variant, else CCW.
    #[inline]
    pub fn rotate_90(self, axis: Axis, clockwise: bool) -> Vector3i {
        match (axis, clockwise) {
            (Axis::X, true) => self.rotate_x_90_cw(),
            (Axis::X, false) => self.rotate_x_90_ccw(),
            (Axis::Y, true) => self.rotate_y_90_cw(),
            (Axis::Y, false) => self.rotate_y_90_ccw(),
            (Axis::Z, true) => self.rotate_z_90_cw(),
            (Axis::Z, false) => self.rotate_z_90_ccw(),
        }
    }

    /// Apply the same 90° rotation to every vector in `vecs` in place.
    /// Matches `math::rotate_90(Span<Vector3i>, Axis, bool)`.
    #[inline]
    pub fn rotate_90_slice(vecs: &mut [Vector3i], axis: Axis, clockwise: bool) {
        for v in vecs.iter_mut() {
            *v = v.rotate_90(axis, clockwise);
        }
    }
}

// ---- Bitwise operators on Vector3i (match vector3i.h operator<< >> & %) ----
// C++ defines these as free operators in the Godot namespace; here they are
// std::ops trait impls so `v << n`, `v >> n`, `v & m`, `v % d` all work directly.

/// Left shift each component by `b`. Matches `operator<<(Vector3i, int)`.
impl core::ops::Shl<u32> for Vector3i {
    type Output = Vector3i;
    #[inline]
    fn shl(self, b: u32) -> Vector3i {
        debug_assert!(b < 32);
        Vector3i::new(self.x << b, self.y << b, self.z << b)
    }
}

/// Arithmetic right shift each component by `b` (sign-extending).
/// Matches `operator>>`. Rust `>>` on `i32` is already arithmetic.
impl core::ops::Shr<u32> for Vector3i {
    type Output = Vector3i;
    #[inline]
    fn shr(self, b: u32) -> Vector3i {
        debug_assert!(b < 32);
        Vector3i::new(self.x >> b, self.y >> b, self.z >> b)
    }
}

/// Bitwise AND of each component with `b`. Matches `operator&(Vector3i, uint32_t)`.
impl core::ops::BitAnd<u32> for Vector3i {
    type Output = Vector3i;
    #[inline]
    fn bitand(self, b: u32) -> Vector3i {
        Vector3i::new(self.x & b as i32, self.y & b as i32, self.z & b as i32)
    }
}

/// Remainder of each component divided by `b`. Matches `operator%(Vector3i, int)`.
impl core::ops::Rem<i32> for Vector3i {
    type Output = Vector3i;
    #[inline]
    fn rem(self, b: i32) -> Vector3i {
        Vector3i::new(self.x % b, self.y % b, self.z % b)
    }
}

// ---- Hash (matches Vector3iHasher using hash_djb2_one_32) ----

impl core::hash::Hash for Vector3i {
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Reproduces the C++ Vector3iHasher chain: djb2 over x, then y, then z.
        // We write a single u32 so equality on equal vectors always hashes equal,
        // independent of the Hasher's own write strategy.
        let mut h = crate::hash::hash_djb2_one_32(self.x as u32, 5381);
        h = crate::hash::hash_djb2_one_32(self.y as u32, h);
        h = crate::hash::hash_djb2_one_32(self.z as u32, h);
        state.write_u32(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_get_returns_vector3_components() {
        let v = Vector3T::new(10, 20, 30);

        assert_eq!(v.get(0), 10);
        assert_eq!(v.get(1), 20);
        assert_eq!(v.get(2), 30);
    }

    #[test]
    fn public_set_updates_vector3_components() {
        let mut v = Vector3T::new(1, 2, 3);

        v.set(0, 10);
        v.set(1, 20);
        v.set(2, 30);

        assert_eq!(v, Vector3T::new(10, 20, 30));
    }

    #[test]
    #[should_panic(expected = "Vector3 index out of range")]
    fn public_get_panics_for_out_of_range_vector3_index() {
        let v = Vector3T::new(1, 2, 3);
        let _ = v.get(3);
    }

    #[test]
    #[should_panic(expected = "Vector3 index out of range")]
    fn public_set_panics_for_out_of_range_vector3_index() {
        let mut v = Vector3T::new(1, 2, 3);
        v.set(usize::MAX, 4);
    }

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
        assert!(math::is_normalized(math::normalized(Vector3f::new(
            3.0, 0.0, 0.0
        ))));
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
        assert_eq!(
            math::get_longest_axis(Vector3f::new(1.0, 2.0, 3.0)),
            Axis::Z
        );
        assert_eq!(
            math::get_longest_axis(Vector3f::new(5.0, 2.0, 3.0)),
            Axis::X
        );
        assert_eq!(
            math::get_longest_axis(Vector3f::new(1.0, 9.0, 3.0)),
            Axis::Y
        );
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

    #[test]
    fn vector3i_util_helpers() {
        assert_eq!(Vector3i::create(7), Vector3i::splat(7));
        assert_eq!(Vector3i::new(3, 4, 5).volume_u64(), 60);
        assert!(Vector3i::new(1, 0, 0).is_unit_vector());
        assert!(!Vector3i::new(1, 1, 0).is_unit_vector());
        assert!(Vector3i::new(2, 2, 2).all_members_equal());
        assert!(!Vector3i::new(2, 2, 3).all_members_equal());
        assert!(Vector3i::new(0, 4, 4).is_empty_size());
        assert!(Vector3i::new(2, 2, 2).is_valid_size());
        assert!(!Vector3i::new(-1, 2, 2).is_valid_size());
    }

    #[test]
    fn vector3i_sort_min_max() {
        let mut a = Vector3i::new(5, 1, 9);
        let mut b = Vector3i::new(2, 8, 3);
        Vector3i::sort_min_max(&mut a, &mut b);
        assert_eq!(a, Vector3i::new(2, 1, 3));
        assert_eq!(b, Vector3i::new(5, 8, 9));
    }

    #[test]
    fn vector3i_zxy_zyx_index_roundtrip() {
        let v = Vector3i::new(1, 2, 3);
        let size = Vector3i::new(10, 10, 10);
        // ZXY: y + sy*(x + sx*z) = 2 + 10*(1 + 10*3) = 2 + 10*31 = 312
        assert_eq!(v.zxy_index(size), 312);
        // ZYX: x + sx*(y + sy*z) = 1 + 10*(2 + 10*3) = 1 + 10*32 = 321
        assert_eq!(v.zyx_index(size), 321);
        // Inverse of ZXY recovers the vector.
        assert_eq!(Vector3i::from_zxy_index(312, size), v);
        // Scalar overload matches vector one.
        assert_eq!(Vector3i::zxy_index_scalars(1, 2, 3, 10, 10), 312);
    }

    #[test]
    fn vector3i_distances_and_dot() {
        let a = Vector3i::new(0, 0, 0);
        let b = Vector3i::new(1, 2, 3);
        assert_eq!(a.manhattan_distance_to(b), 6);
        assert_eq!(a.chebyshev_distance_to(b), 3);
        assert_eq!(Vector3i::new(1, 2, 3).dot(Vector3i::new(4, 5, 6)), 32);
    }

    #[test]
    fn vector3i_min_max_clamp_abs() {
        assert_eq!(
            Vector3i::new(1, 5, 3).min(Vector3i::new(4, 2, 6)),
            Vector3i::new(1, 2, 3)
        );
        assert_eq!(
            Vector3i::new(1, 5, 3).max(Vector3i::new(4, 2, 6)),
            Vector3i::new(4, 5, 6)
        );
        assert_eq!(
            Vector3i::new(10, 10, 10).clamp(Vector3i::new(2, 2, 2), Vector3i::new(5, 5, 5)),
            Vector3i::new(5, 5, 5)
        );
        assert_eq!(Vector3i::new(-1, 2, -3).abs(), Vector3i::new(1, 2, 3));
    }

    #[test]
    fn vector3i_div_helpers() {
        let v = Vector3i::new(-1, 7, 4);
        assert_eq!(v.floordiv_scalar(3), Vector3i::new(-1, 2, 1));
        assert_eq!(v.ceildiv_scalar(3), Vector3i::new(0, 3, 2));
        assert_eq!(
            Vector3i::new(5, 5, 5).wrap(Vector3i::new(3, 3, 3)),
            Vector3i::new(2, 2, 2)
        );
    }

    #[test]
    fn vector3i_rotate_90_axis_dispatch() {
        let x = Vector3i::new(1, 0, 0);
        // Rotating +X around Y by 90 CW sends +X to +Z (matches the f32 test).
        assert_eq!(x.rotate_90(Axis::Y, true), Vector3i::new(0, 0, 1));
        assert_eq!(x.rotate_90(Axis::Y, false), Vector3i::new(0, 0, -1));
        // Direct helpers agree with the dispatch.
        assert_eq!(x.rotate_y_90_cw(), x.rotate_90(Axis::Y, true));
        assert_eq!(x.rotate_y_90_ccw(), x.rotate_90(Axis::Y, false));
        // Slice variant applies to all elements.
        let mut vs = [Vector3i::new(1, 0, 0), Vector3i::new(0, 1, 0)];
        Vector3i::rotate_90_slice(&mut vs, Axis::Z, true);
        assert_eq!(vs[0], Vector3i::new(0, -1, 0));
        assert_eq!(vs[1], Vector3i::new(1, 0, 0));
    }

    #[test]
    fn vector3i_bitwise_ops() {
        let v = Vector3i::new(1, 2, 3);
        assert_eq!(v << 1, Vector3i::new(2, 4, 6));
        assert_eq!(Vector3i::new(-4, 8, -16) >> 1, Vector3i::new(-2, 4, -8)); // arithmetic shift
        assert_eq!(Vector3i::new(7, 7, 7) & 3u32, Vector3i::new(3, 3, 3));
        assert_eq!(Vector3i::new(7, 8, 9) % 3, Vector3i::new(1, 2, 0));
    }

    #[test]
    fn vector3i_hash_matches_djb2_chain() {
        // Equal vectors must hash equal; distinct vectors very likely differ.
        use core::hash::{BuildHasherDefault, Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        let hash_of = |v: Vector3i| -> u64 {
            let mut h = DefaultHasher::new();
            v.hash(&mut h);
            h.finish()
        };
        assert_eq!(
            hash_of(Vector3i::new(1, 2, 3)),
            hash_of(Vector3i::new(1, 2, 3))
        );
        assert_ne!(
            hash_of(Vector3i::new(1, 2, 3)),
            hash_of(Vector3i::new(3, 2, 1))
        );
        // Sanity: also works inside a HashMap (uses our Hash impl).
        let mut map =
            std::collections::HashMap::<Vector3i, i32, BuildHasherDefault<DefaultHasher>>::default(
            );
        map.insert(Vector3i::new(1, 2, 3), 42);
        assert_eq!(map.get(&Vector3i::new(1, 2, 3)), Some(&42));
    }
}
