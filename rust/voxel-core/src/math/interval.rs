//! Interval arithmetic (float instantiation).
//!
//! Faithful port of `util/math/interval.h`. The C++ template is
//! `IntervalT<T>`; the engine uses it as `Interval = IntervalT<real_t>`, which
//! in this port is `f32` (matching [`super::vector3::Vector3f`]). We keep the
//! generic [`IntervalT<T: Copy + PartialOrd>]` shell for layout/fidelity and
//! implement all behavior on the `f32` alias [`Interval`].
//!
//! Interval arithmetic tracks the min/max range an expression can take, which
//! the voxel graph generator uses to bound SDF output across a whole block.

use super::constants::PI_F32;
use super::funcs;

/// A closed `[min, max]` interval. Generic shell matching `IntervalT<T>`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct IntervalT<T: Copy + PartialOrd> {
    /// Inclusive lower bound.
    pub min: T,
    /// Inclusive upper bound.
    pub max: T,
}

/// 2-component interval (per-axis ranges), matching `Interval2T<T>`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Interval2T<T: Copy + PartialOrd> {
    pub x: IntervalT<T>,
    pub y: IntervalT<T>,
}

/// 3-component interval (per-axis ranges), matching `Interval3T<T>`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Interval3T<T: Copy + PartialOrd> {
    pub x: IntervalT<T>,
    pub y: IntervalT<T>,
    pub z: IntervalT<T>,
}

/// Float interval (the engine's `Interval = IntervalT<real_t>`).
pub type Interval = IntervalT<f32>;
pub type Interval2 = Interval2T<f32>;
pub type Interval3 = Interval3T<f32>;

/// An optional secondary interval, returned by [`atan2`] when the result wraps
/// the ±π boundary and must be split. Matches `OptionalIntervalT<T>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OptionalInterval {
    pub value: Interval,
    pub valid: bool,
}

impl Interval {
    /// Create `[min, max]`. In debug builds, asserts `min <= max`.
    pub fn new(min: f32, max: f32) -> Self {
        debug_assert!(min <= max, "Interval min > max: {min} > {max}");
        Self { min, max }
    }

    /// `[v, v]`. Matches `from_single_value`.
    #[inline]
    pub fn single(v: f32) -> Self {
        Self { min: v, max: v }
    }

    /// `min(a,b)..max(a,b)`. Matches `from_unordered_values`.
    #[inline]
    pub fn from_unordered(a: f32, b: f32) -> Self {
        Self::new(funcs::min(a, b), funcs::max(a, b))
    }

    /// `-∞..+∞`. Matches `from_infinity`.
    #[inline]
    pub fn infinity() -> Self {
        Self {
            min: f32::NEG_INFINITY,
            max: f32::INFINITY,
        }
    }

    /// Union of two intervals. Matches `from_union`.
    #[inline]
    pub fn union(a: Interval, b: Interval) -> Self {
        Self::new(funcs::min(a.min, b.min), funcs::max(a.max, b.max))
    }

    #[inline]
    pub fn contains_value(self, v: f32) -> bool {
        v >= self.min && v <= self.max
    }

    #[inline]
    pub fn contains_interval(self, other: Interval) -> bool {
        other.min >= self.min && other.max <= self.max
    }

    #[inline]
    pub fn is_single_value(self) -> bool {
        self.min == self.max
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self.min <= self.max
    }

    /// Extend to include `x`. Matches `add_point`.
    #[inline]
    pub fn add_point(&mut self, x: f32) {
        if x < self.min {
            self.min = x;
        } else if x > self.max {
            self.max = x;
        }
    }

    /// Grow both ends by `e`. Matches `padded`.
    #[inline]
    pub fn padded(self, e: f32) -> Interval {
        Self::new(self.min - e, self.max + e)
    }

    /// Extend to cover `other`. Matches `add_interval`.
    #[inline]
    pub fn add_interval(&mut self, other: Interval) {
        self.add_point(other.min);
        self.add_point(other.max);
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.max - self.min
    }
}

// ---- Operators (match the C++ overloaded operators) ----

impl core::ops::Add<f32> for Interval {
    type Output = Interval;
    #[inline]
    fn add(self, x: f32) -> Interval {
        Self::new(self.min + x, self.max + x)
    }
}

impl core::ops::Add<Interval> for Interval {
    type Output = Interval;
    #[inline]
    fn add(self, other: Interval) -> Interval {
        Self::new(self.min + other.min, self.max + other.max)
    }
}

impl core::ops::Sub<f32> for Interval {
    type Output = Interval;
    #[inline]
    fn sub(self, x: f32) -> Interval {
        Self::new(self.min - x, self.max - x)
    }
}

impl core::ops::Sub<Interval> for Interval {
    type Output = Interval;
    #[inline]
    fn sub(self, other: Interval) -> Interval {
        Self::new(self.min - other.max, self.max - other.min)
    }
}

impl core::ops::Neg for Interval {
    type Output = Interval;
    #[inline]
    fn neg(self) -> Interval {
        Self::new(-self.max, -self.min)
    }
}

impl core::ops::Mul<f32> for Interval {
    type Output = Interval;
    #[inline]
    fn mul(self, x: f32) -> Interval {
        let a = self.min * x;
        let b = self.max * x;
        Self::from_unordered(a, b)
    }
}

impl core::ops::Mul<Interval> for Interval {
    type Output = Interval;
    #[inline]
    fn mul(self, other: Interval) -> Interval {
        let a = self.min * other.min;
        let b = self.min * other.max;
        let c = self.max * other.min;
        let d = self.max * other.max;
        Self::new(min4(a, b, c, d), max4(a, b, c, d))
    }
}

// scalar * interval (commutative, matches the free operator*).
impl core::ops::Mul<Interval> for f32 {
    type Output = Interval;
    #[inline]
    fn mul(self, i: Interval) -> Interval {
        i * self
    }
}

impl core::ops::Div<Interval> for Interval {
    type Output = Interval;
    #[inline]
    fn div(self, other: Interval) -> Interval {
        if other.is_single_value() && other.min == 0.0 {
            // Division by zero. In the voxel graph this returns 0.
            return Interval::single(0.0);
        }
        if other.contains_value(0.0) {
            return Interval::infinity();
        }
        let a = self.min / other.min;
        let b = self.min / other.max;
        let c = self.max / other.min;
        let d = self.max / other.max;
        Self::new(min4(a, b, c, d), max4(a, b, c, d))
    }
}

impl core::ops::Div<f32> for Interval {
    type Output = Interval;
    #[inline]
    fn div(self, x: f32) -> Interval {
        self * (1.0 / x)
    }
}

// ---- Interval math functions (match the free functions in interval.h) ----

#[inline]
fn min4(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a.min(b).min(c).min(d)
}
#[inline]
fn max4(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a.max(b).max(c).max(d)
}

/// `min` lifted to intervals. Matches `min_interval(Interval, Interval)`.
#[inline]
pub fn min_interval(a: Interval, b: Interval) -> Interval {
    Interval::new(funcs::min(a.min, b.min), funcs::min(a.max, b.max))
}

/// `max` lifted to intervals. Matches `max_interval(Interval, Interval)`.
#[inline]
pub fn max_interval(a: Interval, b: Interval) -> Interval {
    Interval::new(funcs::max(a.min, b.min), funcs::max(a.max, b.max))
}

/// `min` of an interval and a scalar. Matches `min_interval(Interval, T)`.
#[inline]
pub fn min_interval_scalar(a: Interval, b: f32) -> Interval {
    Interval::new(funcs::min(a.min, b), funcs::min(a.max, b))
}

/// `max` of an interval and a scalar. Matches `max_interval(Interval, T)`.
#[inline]
pub fn max_interval_scalar(a: Interval, b: f32) -> Interval {
    Interval::new(funcs::max(a.min, b), funcs::max(a.max, b))
}

/// `sqrt` (negative bounds clamped to 0). Matches `sqrt`.
#[inline]
pub fn sqrt(i: Interval) -> Interval {
    Interval::new(
        funcs::sqrt_f32(funcs::max(i.min, 0.0)),
        funcs::sqrt_f32(funcs::max(i.max, 0.0)),
    )
}

/// `abs` over an interval. Matches `abs`.
#[inline]
pub fn abs(i: Interval) -> Interval {
    let lo = if i.contains_value(0.0) {
        0.0
    } else {
        funcs::min(funcs::abs_f32(i.min), funcs::abs_f32(i.max))
    };
    Interval::new(lo, funcs::max(funcs::abs_f32(i.min), funcs::abs_f32(i.max)))
}

/// `clamp` over an interval. Matches `clamp`.
pub fn clamp(i: Interval, p_min: Interval, p_max: Interval) -> Interval {
    if p_min.is_single_value() && p_max.is_single_value() {
        return Interval::new(
            funcs::clamp(i.min, p_min.min, p_max.min),
            funcs::clamp(i.max, p_min.min, p_max.min),
        );
    }
    if i.min >= p_min.max && i.max <= p_max.min {
        return i;
    }
    if i.min >= p_max.max {
        return Interval::single(p_max.max);
    }
    if i.max <= p_min.min {
        return Interval::single(p_min.min);
    }
    Interval::new(p_min.min, p_max.max)
}

/// `lerp` over intervals. Matches `lerp`.
pub fn lerp(a: Interval, b: Interval, t: Interval) -> Interval {
    if t.is_single_value() {
        return Interval::new(
            funcs::lerp_f32(a.min, b.min, t.min),
            funcs::lerp_f32(a.max, b.max, t.min),
        );
    }
    let v0 = a.min + t.min * (b.min - a.min);
    let v1 = a.max + t.min * (b.min - a.max);
    let v2 = a.min + t.max * (b.min - a.min);
    let v3 = a.max + t.max * (b.min - a.max);
    let v4 = a.min + t.min * (b.max - a.min);
    let v5 = a.max + t.min * (b.max - a.max);
    let v6 = a.min + t.max * (b.max - a.min);
    let v7 = a.max + t.max * (b.max - a.max);
    let lo = v0.min(v1).min(v2).min(v3).min(v4).min(v5).min(v6).min(v7);
    let hi = v0.max(v1).max(v2).max(v3).max(v4).max(v5).max(v6).max(v7);
    Interval::new(lo, hi)
}

/// `sin` over an interval. Matches `sin` (simplified to `[-1, 1]` for a range).
#[inline]
pub fn sin(i: Interval) -> Interval {
    if i.is_single_value() {
        Interval::single(funcs::sin_f32(i.min))
    } else {
        Interval::new(-1.0, 1.0)
    }
}

/// `atan` over an interval (monotonic). Matches `atan`.
#[inline]
pub fn atan(t: Interval) -> Interval {
    if t.is_single_value() {
        Interval::single(t.min.atan())
    } else {
        Interval::new(t.min.atan(), t.max.atan())
    }
}

/// `floor` (monotonic). Matches `floor`.
#[inline]
pub fn floor(i: Interval) -> Interval {
    Interval::new(funcs::floor_f32(i.min), funcs::floor_f32(i.max))
}

/// `round` (half-up). Matches `round`.
#[inline]
pub fn round(i: Interval) -> Interval {
    Interval::new(funcs::floor_f32(i.min + 0.5), funcs::floor_f32(i.max + 0.5))
}

/// `snapped` to a step. Matches `snapped`.
#[inline]
pub fn snapped(value: Interval, step: Interval) -> Interval {
    floor(value / step + Interval::single(0.5)) * step
}

/// `wrapf`. Matches `wrapf`.
#[inline]
pub fn wrapf(x: Interval, d: Interval) -> Interval {
    x - d * floor(x / d)
}

/// `smoothstep` over an interval (monotonic). Matches `smoothstep`.
pub fn smoothstep(from: f32, to: f32, weight: Interval) -> Interval {
    if funcs::is_equal_approx(from, to) {
        return Interval::single(from);
    }
    let v0 = funcs::smoothstep_f32(from, to, weight.min);
    let v1 = funcs::smoothstep_f32(from, to, weight.max);
    Interval::from_unordered(v0, v1)
}

/// `x*x` tightened (the interval may straddle 0). Matches `squared`.
pub fn squared(x: Interval) -> Interval {
    if x.min < 0.0 && x.max > 0.0 {
        Interval::new(0.0, funcs::max(x.min * x.min, x.max * x.max))
    } else if x.max <= 0.0 {
        Interval::new(x.max * x.max, x.min * x.min)
    } else {
        Interval::new(x.min * x.min, x.max * x.max)
    }
}

/// `x*x*x` (monotonic ascending). Matches `cubed`.
#[inline]
pub fn cubed(x: Interval) -> Interval {
    Interval::new(x.min * x.min * x.min, x.max * x.max * x.max)
}

/// `a*x² + b*x + c` tightened at the parabola tip. Matches `polynomial_second_degree`.
pub fn polynomial_second_degree(x: Interval, a: f32, b: f32, c: f32) -> Interval {
    if a == 0.0 {
        if b == 0.0 {
            return Interval::single(c);
        } else {
            return b * x + c;
        }
    }
    let parabola_x = -b / (2.0 * a);
    let y0 = a * x.min * x.min + b * x.min + c;
    let y1 = a * x.max * x.max + b * x.max + c;
    if x.min < parabola_x && x.max > parabola_x {
        let parabola_y = a * parabola_x * parabola_x + b * parabola_x + c;
        if a < 0.0 {
            return Interval::new(funcs::min(y0, y1), parabola_y);
        } else {
            return Interval::new(parabola_y, funcs::max(y0, y1));
        }
    }
    if (a >= 0.0 && x.min >= parabola_x) || (a < 0.0 && x.max < parabola_x) {
        Interval::new(y0, y1)
    } else {
        Interval::new(y1, y0)
    }
}

/// Length of a 2D vector given per-axis intervals. Matches `get_length(x, y)`.
#[inline]
pub fn get_length2(x: Interval, y: Interval) -> Interval {
    sqrt(squared(x) + squared(y))
}

/// Length of a 3D vector given per-axis intervals. Matches `get_length(x, y, z)`.
#[inline]
pub fn get_length3(x: Interval, y: Interval, z: Interval) -> Interval {
    sqrt(squared(x) + squared(y) + squared(z))
}

/// Integer power of an interval. Matches `powi`.
pub fn powi(x: Interval, pi: i32) -> Interval {
    if pi >= 0 {
        let pf = pi as f32;
        if pi % 2 == 1 {
            // Odd: ascending.
            Interval::new(funcs::pow_f32(x.min, pf), funcs::pow_f32(x.max, pf))
        } else if x.min < 0.0 && x.max > 0.0 {
            Interval::new(
                0.0,
                funcs::max(funcs::pow_f32(x.min, pf), funcs::pow_f32(x.max, pf)),
            )
        } else if x.max <= 0.0 {
            Interval::new(funcs::pow_f32(x.max, pf), funcs::pow_f32(x.min, pf))
        } else {
            Interval::new(funcs::pow_f32(x.min, pf), funcs::pow_f32(x.max, pf))
        }
    } else {
        // Negative integer powers not implemented.
        Interval::infinity()
    }
}

/// Float power of an interval. Matches `pow(Interval, float)`.
pub fn pow(x: Interval, pf: f32) -> Interval {
    let pi = pf as i32;
    if funcs::is_equal_approx(pi as f32, pf) {
        powi(x, pi)
    } else {
        Interval::infinity()
    }
}

/// Pure computation behind [`atan2`]: returns the primary interval plus, when the
/// result wraps the ±π boundary (Q1↔Q2 crossing), the secondary interval that
/// splits it. Separated from [`atan2`] so the optional output sink is touched once.
fn atan2_compute(y: Interval, x: Interval) -> (Interval, Option<Interval>) {
    if y.is_single_value() && x.is_single_value() {
        return (Interval::single(funcs::atan2_f32(y.min, x.max)), None);
    }

    let in_nx = x.min <= 0.0;
    let in_px = x.max >= 0.0;
    let in_ny = y.min <= 0.0;
    let in_py = y.max >= 0.0;

    if in_nx && in_px && in_ny && in_py {
        return (Interval::new(-PI_F32, PI_F32), None);
    }

    let in_q0 = in_px && in_py;
    let in_q1 = in_nx && in_py;
    let in_q2 = in_nx && in_ny;
    let in_q3 = in_px && in_ny;

    if in_q0 && in_q1 {
        return (
            Interval::new(
                funcs::atan2_f32(y.min, x.max),
                funcs::atan2_f32(y.min, x.min),
            ),
            None,
        );
    }
    if in_q1 && in_q2 {
        // Crossing Q1↔Q2 wraps the angle from +π to -π; the result splits in two.
        let primary = Interval::new(-PI_F32, funcs::atan2_f32(y.min, x.max));
        let secondary = Interval::new(funcs::atan2_f32(y.max, x.max), PI_F32);
        return (primary, Some(secondary));
    }
    if in_q2 && in_q3 {
        return (
            Interval::new(
                funcs::atan2_f32(y.max, x.min),
                funcs::atan2_f32(y.max, x.max),
            ),
            None,
        );
    }
    if in_q3 && in_q0 {
        return (
            Interval::new(
                funcs::atan2_f32(y.min, x.min),
                funcs::atan2_f32(y.max, x.min),
            ),
            None,
        );
    }

    if in_q0 {
        return (
            Interval::new(
                funcs::atan2_f32(y.min, x.max),
                funcs::atan2_f32(y.max, x.min),
            ),
            None,
        );
    }
    if in_q1 {
        return (
            Interval::new(
                funcs::atan2_f32(y.max, x.max),
                funcs::atan2_f32(y.min, x.min),
            ),
            None,
        );
    }
    if in_q2 {
        return (
            Interval::new(
                funcs::atan2_f32(y.max, x.min),
                funcs::atan2_f32(y.min, x.max),
            ),
            None,
        );
    }
    if in_q3 {
        return (
            Interval::new(
                funcs::atan2_f32(y.min, x.min),
                funcs::atan2_f32(y.max, x.max),
            ),
            None,
        );
    }

    (Interval::new(-PI_F32, PI_F32), None)
}

/// `atan2(y, x)` over intervals, handling the ±π wrap. Matches `atan2`.
/// If `secondary_output` is given and the result must split across the wrap, it
/// receives the second interval and `valid` is set. Without a sink, a wrap
/// collapses to the full `[-π, π]` range (matching the C++ null-pointer path).
pub fn atan2(
    y: Interval,
    x: Interval,
    secondary_output: Option<&mut OptionalInterval>,
) -> Interval {
    let (primary, split) = atan2_compute(y, x);
    match (secondary_output, split) {
        (Some(sec), Some(secondary_value)) => {
            sec.value = secondary_value;
            sec.valid = true;
            primary
        }
        (Some(sec), None) => {
            sec.valid = false;
            primary
        }
        (None, Some(_)) => Interval::new(-PI_F32, PI_F32),
        (None, None) => primary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sub_mul_scalar() {
        let a = Interval::new(1.0, 2.0);
        assert_eq!((a + 3.0).max, 5.0);
        assert_eq!((a - 3.0).min, -2.0);
        // Multiplying by a negative flips the bounds.
        let m = a * -1.0;
        assert_eq!(m.min, -2.0);
        assert_eq!(m.max, -1.0);
    }

    #[test]
    fn interval_mul_interval() {
        // [-2,1] * [-1,3] -> min(-2*-1,-2*3,1*-1,1*3)=-6 ... max=6? -2*3=-6, 1*3=3,
        // -2*-1=2, 1*-1=-1 => min -6, max 3.
        let r = Interval::new(-2.0, 1.0) * Interval::new(-1.0, 3.0);
        assert_eq!(r.min, -6.0);
        assert_eq!(r.max, 3.0);
    }

    #[test]
    fn division_by_zero_returns_zero() {
        let r = Interval::new(1.0, 2.0) / Interval::single(0.0);
        assert!(r.is_single_value() && r.min == 0.0);
    }

    #[test]
    fn division_straddling_zero_is_infinity() {
        let r = Interval::new(1.0, 2.0) / Interval::new(-1.0, 1.0);
        assert!(r.min.is_infinite() && r.max.is_infinite());
    }

    #[test]
    fn squared_straddling_zero() {
        // [-1, 2]² includes 0 -> [0, max(1,4)] = [0,4].
        let r = squared(Interval::new(-1.0, 2.0));
        assert_eq!(r.min, 0.0);
        assert_eq!(r.max, 4.0);
    }

    #[test]
    fn sqrt_clamps_negative() {
        let r = sqrt(Interval::new(-4.0, 9.0));
        assert!((r.min - 0.0).abs() < 1e-5);
        assert!((r.max - 3.0).abs() < 1e-5);
    }

    #[test]
    fn union_and_contains() {
        let u = Interval::union(Interval::new(1.0, 3.0), Interval::new(5.0, 7.0));
        assert_eq!(u.min, 1.0);
        assert_eq!(u.max, 7.0);
        assert!(u.contains_value(2.0));
        assert!(u.contains_interval(Interval::new(2.0, 6.0)));
    }

    #[test]
    fn min_max_interval_scalar() {
        let a = Interval::new(1.0, 5.0);
        assert_eq!(min_interval_scalar(a, 3.0), Interval::new(1.0, 3.0));
        assert_eq!(max_interval_scalar(a, 3.0), Interval::new(3.0, 5.0));
    }

    #[test]
    fn polynomial_second_degree_tip() {
        // x² over x=[-1,2]: tip at 0 (a=1>0), so [0, max(1,4)] = [0,4].
        let r = polynomial_second_degree(Interval::new(-1.0, 2.0), 1.0, 0.0, 0.0);
        assert!((r.min - 0.0).abs() < 1e-5);
        assert!((r.max - 4.0).abs() < 1e-5);
    }

    #[test]
    fn atan2_single_quadrant() {
        // First quadrant only: result stays in [0, π/2].
        let r = atan2(Interval::new(1.0, 2.0), Interval::new(1.0, 2.0), None);
        assert!(r.min >= -1e-5 && r.max <= PI_F32 / 2.0 + 1e-5);
    }
}
