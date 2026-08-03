//! Scalar math functions.
//!
//! Ported from `util/math/funcs.h`.
//!
//! Mirrors Godot's `Math::*` API surface used by godot_voxel. Float behavior
//! matches C++ exactly (same `f32::floor`, `f32::sqrt` intrinsics on IEEE-754).

use super::constants::UNIT_EPSILON;

#[inline]
pub fn min<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}

#[inline]
pub fn max<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}

#[inline]
pub fn min3<T: PartialOrd + Copy>(a: T, b: T, c: T) -> T {
    min(min(a, b), c)
}

#[inline]
pub fn max3<T: PartialOrd + Copy>(a: T, b: T, c: T) -> T {
    max(max(a, b), c)
}

#[inline]
pub fn clamp<T: PartialOrd + Copy>(x: T, lo: T, hi: T) -> T {
    // Matches C++: min(max(x, lo), hi) — note: does NOT enforce lo <= hi.
    min(max(x, lo), hi)
}

#[inline]
pub fn clampf(x: f32, lo: f32, hi: f32) -> f32 {
    clamp(x, lo, hi)
}

/// Clip a 1D half-open range `[pos, pos+size)` into `[lim_pos, lim_pos+lim_size)`.
/// Shared by `Box2i`/`Box3i::clip_range`. Mutates `pos`/`size` in place; clamps
/// the resulting size to be non-negative.
#[inline]
pub fn clip_range(pos: &mut i32, size: &mut i32, lim_pos: i32, lim_size: i32) {
    let mut max_pos = *pos + *size;
    let lim_max_pos = lim_pos + lim_size;

    *pos = clamp(*pos, lim_pos, lim_max_pos);
    max_pos = clamp(max_pos, lim_pos, lim_max_pos);

    *size = max_pos - *pos;
    if *size < 0 {
        *size = 0;
    }
}

#[inline]
pub fn squared<T: Copy + core::ops::Mul<Output = T>>(x: T) -> T {
    x * x
}

#[inline]
pub fn cubed<T: Copy + core::ops::Mul<Output = T>>(x: T) -> T {
    x * x * x
}

#[inline]
pub fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn lerp_f64(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Euclidean (floored) integer division. Expects a strictly positive divisor.
/// Matches C++ `floordiv(int x, int d)`.
#[inline]
pub fn floordiv(x: i32, d: i32) -> i32 {
    debug_assert!(d > 0);
    if x < 0 {
        (x - d + 1) / d
    } else {
        x / d
    }
}

/// Ceiling integer division. Expects a strictly positive divisor.
#[inline]
#[allow(clippy::manual_div_ceil)] // `div_ceil` is unstable on this stable toolchain
pub fn ceildiv(x: i32, d: i32) -> i32 {
    debug_assert!(d > 0);
    if x > 0 {
        (x + d - 1) / d
    } else {
        x / d
    }
}

#[inline]
#[allow(clippy::manual_div_ceil)] // `div_ceil` is unstable on this stable toolchain
pub fn ceildiv_u32(x: u32, d: u32) -> u32 {
    debug_assert!(d > 0);
    (x + d - 1) / d
}

/// `Math::wrapi` with zero min.
#[inline]
pub fn wrap_i32(x: i32, d: i32) -> i32 {
    debug_assert!(d > 0);
    ((x % d) + d) % d
}

/// `Math::wrapf` with zero min.
#[inline]
pub fn wrapf_f32(x: f32, d: f32) -> f32 {
    if is_zero_approx(d) {
        0.0
    } else {
        x - (d * f32::floor(x / d))
    }
}

#[inline]
pub fn smoothstep_f32(from: f32, to: f32, weight: f32) -> f32 {
    if is_equal_approx(from, to) {
        return from;
    }
    let x = clamp((weight - from) / (to - from), 0.0f32, 1.0f32);
    x * x * (3.0 - 2.0 * x)
}

#[inline]
pub fn fract_f32(x: f32) -> f32 {
    x - f32::floor(x)
}

#[inline]
pub fn is_power_of_two(x: usize) -> bool {
    x != 0 && (x & (x.wrapping_sub(1))) == 0
}

#[inline]
pub fn get_next_power_of_two_32(x: u32) -> u32 {
    if x == 0 {
        return 0;
    }
    let mut x = x - 1;
    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x |= x >> 8;
    x |= x >> 16;
    x + 1
}

#[inline]
pub fn get_previous_power_of_two_32(mut x: u32) -> u32 {
    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x |= x >> 8;
    x |= x >> 16;
    x - (x >> 1)
}

/// Assuming `pot == (1 << i)`, returns `i`. Panics if not a power of two.
#[inline]
pub fn get_shift_from_power_of_two_32(pot: u32) -> u32 {
    debug_assert!(is_power_of_two(pot as usize));
    for i in 0..32u32 {
        if pot == (1u32 << i) {
            return i;
        }
    }
    panic!("Input was not a valid power of two");
}

/// Align `a` up to `align` (power of two).
#[inline]
pub fn alignup(a: usize, align: usize) -> usize {
    debug_assert!(is_power_of_two(align));
    (a + align - 1) & !(align - 1)
}

// ---- Float comparison (Godot semantics) ----

#[inline]
pub fn is_equal_approx(a: f32, b: f32) -> bool {
    // Godot Math::is_equal_approx
    if a == b {
        return true;
    }
    let tolerance = UNIT_EPSILON * max(a.abs(), b.abs());
    (a - b).abs() < tolerance
}

#[inline]
pub fn is_equal_approx_tol(a: f32, b: f32, tolerance: f32) -> bool {
    if a == b {
        return true;
    }
    (a - b).abs() <= tolerance
}

#[inline]
pub fn is_zero_approx(s: f32) -> bool {
    s.abs() < UNIT_EPSILON
}

#[inline]
pub fn is_nan_f32(x: f32) -> bool {
    x.is_nan()
}

#[inline]
pub fn is_inf_f32(x: f32) -> bool {
    x.is_infinite()
}

/// `Math::snapped` (float variant).
#[inline]
pub fn snappedf(value: f32, step: f32) -> f32 {
    if step != 0.0 {
        f32::floor(value / step + 0.5) * step
    } else {
        value
    }
}

/// Returns -1 if `x` is negative, and 1 otherwise. Returns 1 (not 0) when `x == 0`.
#[inline]
pub fn sign_nonzero_f32(x: f32) -> f32 {
    if x < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// Returns -1 if `x` is negative, and 1 otherwise. Returns 1 (not 0) when `x == 0`.
#[inline]
pub fn sign_nonzero_i32(x: i32) -> i32 {
    if x < 0 {
        -1
    } else {
        1
    }
}

#[inline]
pub fn sign_f32(v: f32) -> f32 {
    if v == 0.0 {
        0.0
    } else if v < 0.0 {
        -1.0
    } else {
        1.0
    }
}

#[inline]
pub fn sign_i32(v: i32) -> i32 {
    if v == 0 {
        0
    } else if v < 0 {
        -1
    } else {
        1
    }
}

#[inline]
pub fn deg_to_rad_f32(deg: f32) -> f32 {
    deg * super::constants::PI_F32 / 180.0
}

#[inline]
pub fn deg_to_rad_f64(deg: f64) -> f64 {
    deg * super::constants::PI_F64 / 180.0
}

/// Parameters for `a*x + b` linear function. Matches `LinearFuncParams` in C++.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LinearFuncParams {
    pub a: f32,
    pub b: f32,
}

/// Given source and destination intervals, returns parameters for `a*x + b` to remap source → dest.
pub fn remap_intervals_to_linear_params(
    min0: f32,
    max0: f32,
    min1: f32,
    max1: f32,
) -> LinearFuncParams {
    if is_equal_approx(max0, min0) {
        return LinearFuncParams { a: 0.0, b: 0.0 };
    }
    let a = (max1 - min1) / (max0 - min0);
    let b = min1 - a * min0;
    LinearFuncParams { a, b }
}

/// Trilinear interpolation between 8 corner values of a unit cube.
/// `p` coordinates are in 0..1 but not clamped (extrapolation possible).
/// Corner naming `vXYZ` where X,Y,Z are 0 or 1 (matches C++ doc diagram).
///
/// Generic over `T` (typically `f32`); scalar × `T` must be defined by the caller's
/// type. For `T = f32` this is just `f32` arithmetic.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn interpolate_trilinear_f32(
    v000: f32,
    v100: f32,
    v101: f32,
    v001: f32,
    v010: f32,
    v110: f32,
    v111: f32,
    v011: f32,
    p: super::vector3::Vector3f,
) -> f32 {
    let v00 = v000 + p.x * (v100 - v000);
    let v10 = v010 + p.x * (v110 - v010);
    let v01 = v001 + p.x * (v101 - v001);
    let v11 = v011 + p.x * (v111 - v011);

    let v0 = v00 + p.y * (v10 - v00);
    let v1 = v01 + p.y * (v11 - v01);

    v0 + p.z * (v1 - v0)
}

/// Checked multiply for u64 (returns 0 on overflow in dev builds).
#[inline]
pub fn multiply_check_overflow_u64(a: u64, b: u64) -> u64 {
    let r = a.wrapping_mul(b);
    debug_assert!(a == 0 || r / a == b, "Multiplication overflow");
    r
}

/// Sort two values in place.
#[inline]
pub fn sort2<T: Ord>(a: &mut T, b: &mut T) {
    if a > b {
        core::mem::swap(a, b);
    }
}

/// Arithmetic right shift (sign-extending). Rust `>>` on signed ints is already arithmetic.
#[inline]
pub fn arithmetic_rshift(a: i32, b: u32) -> i32 {
    a >> b
}

// ---- Scalar math forwarding (Math:: namespace) ----

#[inline]
pub fn floor_f32(x: f32) -> f32 {
    f32::floor(x)
}
#[inline]
pub fn ceil_f32(x: f32) -> f32 {
    f32::ceil(x)
}
#[inline]
pub fn sqrt_f32(x: f32) -> f32 {
    f32::sqrt(x)
}
#[inline]
pub fn abs_f32(x: f32) -> f32 {
    f32::abs(x)
}
#[inline]
pub fn abs_i32(x: i32) -> i32 {
    i32::abs(x)
}
#[inline]
pub fn sin_f32(x: f32) -> f32 {
    f32::sin(x)
}
#[inline]
pub fn cos_f32(x: f32) -> f32 {
    f32::cos(x)
}
#[inline]
pub fn tan_f32(x: f32) -> f32 {
    f32::tan(x)
}
#[inline]
pub fn atan2_f32(y: f32, x: f32) -> f32 {
    f32::atan2(y, x)
}
#[inline]
pub fn pow_f32(x: f32, y: f32) -> f32 {
    f32::powf(x, y)
}
#[inline]
pub fn exp_f32(x: f32) -> f32 {
    f32::exp(x)
}
#[inline]
pub fn log_f32(x: f32) -> f32 {
    f32::ln(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floordiv_matches_table() {
        // Mirrors the doc table in funcs.h for divisor 3.
        assert_eq!(floordiv(-6, 3), -2);
        assert_eq!(floordiv(-5, 3), -2);
        assert_eq!(floordiv(-4, 3), -2);
        assert_eq!(floordiv(-3, 3), -1);
        assert_eq!(floordiv(-2, 3), -1);
        assert_eq!(floordiv(-1, 3), -1);
        assert_eq!(floordiv(0, 3), 0);
        assert_eq!(floordiv(1, 3), 0);
        assert_eq!(floordiv(2, 3), 0);
        assert_eq!(floordiv(3, 3), 1);
        assert_eq!(floordiv(4, 3), 1);
        assert_eq!(floordiv(5, 3), 1);
        assert_eq!(floordiv(6, 3), 2);
    }

    #[test]
    fn ceildiv_matches_table() {
        assert_eq!(ceildiv(0, 10), 0);
        assert_eq!(ceildiv(1, 10), 1);
        assert_eq!(ceildiv(5, 10), 1);
        assert_eq!(ceildiv(10, 10), 1);
        assert_eq!(ceildiv(11, 10), 2);
    }

    #[test]
    fn next_power_of_two() {
        assert_eq!(get_next_power_of_two_32(0), 0);
        assert_eq!(get_next_power_of_two_32(1), 1);
        assert_eq!(get_next_power_of_two_32(3), 4);
        assert_eq!(get_next_power_of_two_32(5), 8);
        assert_eq!(get_next_power_of_two_32(33), 64);
    }

    #[test]
    fn previous_power_of_two() {
        assert_eq!(get_previous_power_of_two_32(0), 0);
        assert_eq!(get_previous_power_of_two_32(1), 1);
        assert_eq!(get_previous_power_of_two_32(7), 4);
        assert_eq!(get_previous_power_of_two_32(9), 8);
    }

    #[test]
    fn wrap_behavior() {
        assert_eq!(wrap_i32(-1, 3), 2);
        assert_eq!(wrap_i32(0, 3), 0);
        assert_eq!(wrap_i32(3, 3), 0);
        assert_eq!(wrap_i32(4, 3), 1);
    }

    #[test]
    fn is_equal_approx_behavior() {
        assert!(is_equal_approx(1.0, 1.0));
        assert!(is_equal_approx(1.0, 1.0 + 1e-8));
        assert!(!is_equal_approx(1.0, 1.1));
        assert!(is_zero_approx(0.0));
        assert!(is_zero_approx(1e-8));
    }

    #[test]
    fn smoothstep_endpoints() {
        assert!((smoothstep_f32(0.0, 1.0, 0.0) - 0.0).abs() < 1e-6);
        assert!((smoothstep_f32(0.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((smoothstep_f32(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
    }
}
