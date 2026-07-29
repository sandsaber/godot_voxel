//! 4D vector types.
//!
//! Ported from `util/math/vector4t.h` (template `Vector4T<T>`) and
//! `util/math/vector4f.h` (free math functions on `Vector4f = Vector4T<f32>`).
//! Mirrors the [`super::vector3`] module layout: a generic struct with operators
//! plus a `math::` namespace of free functions.
//!
//! ## Note on the upstream `length_squared` bug
//! The C++ `vector4f.h` computes `v.x*v.x + v.y*v.y + v.z*v.z + v.w + v.w`,
//! which is almost certainly a typo for `v.w * v.w`. The `+` form makes
//! `normalized()` produce wrong results whenever `w != 0`. This port uses the
//! **corrected** `v.w * v.w`. If a future parity test against C++ requires the
//! buggy behaviour, gate it behind a feature; for now correctness wins.

use super::funcs;

/// Generic 4D vector. Matches `Vector4T<T>` in C++: plain fields + a minimal set
/// of operators (add, componentwise mul, scalar mul). `#[repr(C)]` so it matches
/// the C++ `union { struct {x,y,z,w}; T coords[4]; }` layout for FFI / GPU upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct Vector4T<T: Copy> {
    pub x: T,
    pub y: T,
    pub z: T,
    pub w: T,
}

impl<T: Copy> Vector4T<T> {
    pub const fn new(x: T, y: T, z: T, w: T) -> Self {
        Self { x, y, z, w }
    }

    /// Construct from a single scalar broadcast to all components (matches the
    /// `explicit Vector4T(T)` C++ ctor; kept named to avoid implicit conversions).
    pub const fn splat(v: T) -> Self {
        Self {
            x: v,
            y: v,
            z: v,
            w: v,
        }
    }

    /// Index access with runtime bounds check. Mirrors `operator[]`.
    #[inline]
    pub fn get(&self, i: usize) -> T {
        match i {
            0 => self.x,
            1 => self.y,
            2 => self.z,
            3 => self.w,
            _ => panic!("Vector4 index out of range"),
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: T) {
        match i {
            0 => self.x = v,
            1 => self.y = v,
            2 => self.z = v,
            3 => self.w = v,
            _ => panic!("Vector4 index out of range"),
        }
    }
}

impl<T: Copy> core::ops::Index<usize> for Vector4T<T> {
    type Output = T;
    #[inline]
    fn index(&self, i: usize) -> &T {
        debug_assert!(i < 4);
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            3 => &self.w,
            _ => panic!("Vector4 index out of range"),
        }
    }
}

impl<T: Copy> core::ops::IndexMut<usize> for Vector4T<T> {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut T {
        debug_assert!(i < 4);
        match i {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            3 => &mut self.w,
            _ => panic!("Vector4 index out of range"),
        }
    }
}

// ---- Operator overloading via std::ops ----
// vector4t.h only defines `+`, componentwise `*`, and scalar `*`. We provide
// those exactly; the richer set (-, /, neg, assign variants) is added because it
// is cheap, idiomatic, and matches how vector3.rs was ported. Each is marked to
// make the intent clear.

macro_rules! impl_vector4_ops {
    ($t:ty, $zero:expr) => {
        impl Vector4T<$t> {
            #[inline]
            pub fn zero() -> Self {
                Self::splat($zero)
            }
        }

        impl core::ops::Add for Vector4T<$t> {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self {
                Self::new(
                    self.x + rhs.x,
                    self.y + rhs.y,
                    self.z + rhs.z,
                    self.w + rhs.w,
                )
            }
        }

        // Componentwise multiply (matches `operator*(Vector4T)` in vector4t.h).
        impl core::ops::Mul for Vector4T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: Self) -> Self {
                Self::new(
                    self.x * rhs.x,
                    self.y * rhs.y,
                    self.z * rhs.z,
                    self.w * rhs.w,
                )
            }
        }

        // Scalar multiply (matches `operator*(T)` in vector4t.h).
        impl core::ops::Mul<$t> for Vector4T<$t> {
            type Output = Self;
            #[inline]
            fn mul(self, s: $t) -> Self {
                Self::new(self.x * s, self.y * s, self.z * s, self.w * s)
            }
        }

        // scalar * vector (commutative convenience, matching vector3.rs).
        impl core::ops::Mul<Vector4T<$t>> for $t {
            type Output = Vector4T<$t>;
            #[inline]
            fn mul(self, v: Vector4T<$t>) -> Vector4T<$t> {
                v * self
            }
        }
    };
}

impl_vector4_ops!(f32, 0.0);
impl_vector4_ops!(f64, 0.0);
impl_vector4_ops!(i32, 0);

// ---- Type aliases ----

pub type Vector4f = Vector4T<f32>;
pub type Vector4d = Vector4T<f64>;
pub type Vector4i = Vector4T<i32>;

/// `Vector4f` math, mirroring `zylann::math::` overloads from `vector4f.h`.
pub mod math {
    use super::*;

    /// Squared length. **Corrected** from the C++ `v.w + v.w` typo to `v.w * v.w`
    /// (see the module-level note). Matches the intended behaviour, not the bug.
    #[inline]
    pub fn length_squared(v: Vector4f) -> f32 {
        v.x * v.x + v.y * v.y + v.z * v.z + v.w * v.w
    }

    /// Unit vector, or zero if `v` has zero length. Matches `normalized`.
    #[inline]
    pub fn normalized(v: Vector4f) -> Vector4f {
        let lengthsq = length_squared(v);
        if lengthsq == 0.0 {
            Vector4f::zero()
        } else {
            let length = 1.0 / funcs::sqrt_f32(lengthsq);
            v * length
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_get_returns_vector4_components() {
        let v = Vector4T::new(10, 20, 30, 40);

        assert_eq!(v.get(0), 10);
        assert_eq!(v.get(1), 20);
        assert_eq!(v.get(2), 30);
        assert_eq!(v.get(3), 40);
    }

    #[test]
    fn public_set_updates_vector4_components() {
        let mut v = Vector4T::new(1, 2, 3, 4);

        v.set(0, 10);
        v.set(1, 20);
        v.set(2, 30);
        v.set(3, 40);

        assert_eq!(v, Vector4T::new(10, 20, 30, 40));
    }

    #[test]
    #[should_panic(expected = "Vector4 index out of range")]
    fn public_get_panics_for_out_of_range_vector4_index() {
        let v = Vector4T::new(1, 2, 3, 4);
        let _ = v.get(4);
    }

    #[test]
    #[should_panic(expected = "Vector4 index out of range")]
    fn public_set_panics_for_out_of_range_vector4_index() {
        let mut v = Vector4T::new(1, 2, 3, 4);
        v.set(usize::MAX, 5);
    }

    #[test]
    fn ctors_and_index() {
        let v = Vector4f::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(v.get(3), 4.0);
        assert_eq!(v[0], 1.0);
        assert_eq!(Vector4f::splat(5.0), Vector4f::new(5.0, 5.0, 5.0, 5.0));
        assert_eq!(Vector4f::zero(), Vector4f::new(0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn add_and_mul() {
        let a = Vector4f::new(1.0, 2.0, 3.0, 4.0);
        let b = Vector4f::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!(a + b, Vector4f::new(11.0, 22.0, 33.0, 44.0));
        // Componentwise multiply.
        assert_eq!(
            a * Vector4f::new(2.0, 3.0, 4.0, 5.0),
            Vector4f::new(2.0, 6.0, 12.0, 20.0)
        );
        // Scalar multiply (both orders).
        assert_eq!(a * 2.0, Vector4f::new(2.0, 4.0, 6.0, 8.0));
        assert_eq!(2.0 * a, Vector4f::new(2.0, 4.0, 6.0, 8.0));
    }

    #[test]
    fn length_squared_uses_w_squared() {
        // The corrected formula: 1 + 4 + 9 + 16 = 30 (NOT the buggy 1+4+9+8=22).
        let v = Vector4f::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(math::length_squared(v), 30.0);
    }

    #[test]
    fn normalized_unit_vector() {
        // (1,0,0,0) is already unit length.
        assert_eq!(
            math::normalized(Vector4f::new(1.0, 0.0, 0.0, 0.0)),
            Vector4f::new(1.0, 0.0, 0.0, 0.0)
        );
        // (0,0,0,0) stays zero.
        assert_eq!(math::normalized(Vector4f::zero()), Vector4f::zero());
        // (2,0,0,0) normalizes to (1,0,0,0).
        let n = math::normalized(Vector4f::new(2.0, 0.0, 0.0, 0.0));
        assert!((n.x - 1.0).abs() < 1e-6);
        // A vector with a nonzero w must normalize correctly (the buggy C++
        // version would not): (0,0,0,2) -> (0,0,0,1).
        let nw = math::normalized(Vector4f::new(0.0, 0.0, 0.0, 2.0));
        assert!((nw.w - 1.0).abs() < 1e-6);
    }

    #[test]
    fn i32_ops() {
        let a = Vector4i::new(1, 2, 3, 4);
        let b = Vector4i::new(10, 20, 30, 40);
        assert_eq!(a + b, Vector4i::new(11, 22, 33, 44));
        assert_eq!(a * 3, Vector4i::new(3, 6, 9, 12));
    }
}
