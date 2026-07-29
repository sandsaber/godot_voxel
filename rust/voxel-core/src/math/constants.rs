//! Math constants.
//!
//! Ported from `util/math/constants.h`.
//!
//! Matches Godot / godot_voxel conventions: `TAU`, `PI`, `SQRT2`, `SQRT3`.

use core::f32;
use core::f64;

pub const TAU_F32: f32 = core::f32::consts::TAU;
pub const INV_TAU_F32: f32 = core::f32::consts::TAU.recip();
pub const PI_F32: f32 = core::f32::consts::PI;
pub const SQRT2_F32: f32 = core::f32::consts::SQRT_2;
pub const SQRT3_F32: f32 = 1.732_050_8f32;

pub const TAU_F64: f64 = core::f64::consts::TAU;
pub const INV_TAU_F64: f64 = core::f64::consts::TAU.recip();
pub const PI_F64: f64 = core::f64::consts::PI;
pub const SQRT2_F64: f64 = core::f64::consts::SQRT_2;
pub const SQRT3_F64: f64 = 1.732_050_807_568_877_2;

/// Epsilon used by Godot for floating point equality (`UNIT_EPSILON`).
pub const UNIT_EPSILON: f32 = 0.00001;

/// Axis index. Matches C++ `enum Axis { AXIS_X = 0, AXIS_Y = 1, AXIS_Z = 2 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Axis {
    X = 0,
    Y = 1,
    Z = 2,
}

impl Axis {
    #[inline]
    pub fn as_index(self) -> usize {
        self as u8 as usize
    }
}
