//! Color types: floating-point [`Color`] (each component 0..1) and 8-bit [`Color8`].
//!
//! Ported from `util/math/color.h` (float `Color` + `lerp`) and
//! `util/math/color8.h` (8-bit `Color8` with packed u8/u16/u32 conversions).
//! `Color` matches Godot's `Color` layout; `Color8` is the lighter storage form
//! used by voxel libraries and texture channels.

use super::funcs;

/// Float RGBA color, each component in 0..1. Matches Godot `Color`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);
}

/// 8-bit RGBA color. Matches C++ `Color8`. Stored as four `u8` fields; the C++
/// `packed_value`/`components` union accessors become [`Color8::to_u32`] /
/// [`Color8::component`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[repr(C)]
pub struct Color8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color8 {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Construct from a float [`Color`] (components * 255, truncated).
    /// Matches `Color8(Color)`.
    pub fn from_color(c: Color) -> Self {
        Self {
            r: (c.r * 255.0) as u8,
            g: (c.g * 255.0) as u8,
            b: (c.b * 255.0) as u8,
            a: (c.a * 255.0) as u8,
        }
    }

    /// Convert to a float [`Color`] (components / 255). Matches `operator Color()`.
    pub fn to_color(self) -> Color {
        Color::new(
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        )
    }

    /// Per-component access by index (matches the C++ `components[4]` union member).
    #[inline]
    pub fn component(self, i: usize) -> u8 {
        match i {
            0 => self.r,
            1 => self.g,
            2 => self.b,
            3 => self.a,
            _ => panic!("Color8 component index out of range"),
        }
    }

    /// Decode `rrggbbaa` (2 bits each, scaled to 0..255 by *85). Matches `from_u8`.
    pub fn from_u8(v: u8) -> Self {
        Self {
            r: (v >> 6) * 85,
            g: ((v >> 4) & 3) * 85,
            b: ((v >> 2) & 3) * 85,
            a: (v & 3) * 85,
        }
    }

    /// Decode `rrrrggggbbbbaaaa` (4 bits each, scaled to 0..255 by *17). Matches `from_u16`.
    pub fn from_u16(v: u16) -> Self {
        Self {
            r: ((v >> 12) as u8) * 17,
            g: (((v >> 8) & 0xf) as u8) * 17,
            b: (((v >> 4) & 0xf) as u8) * 17,
            a: ((v & 0xf) as u8) * 17,
        }
    }

    /// Decode `rrrrrrrrggggggggbbbbbbbbaaaaaaaa`. Matches `from_u32`.
    pub fn from_u32(c: u32) -> Self {
        Self {
            r: (c >> 24) as u8,
            g: ((c >> 16) & 0xff) as u8,
            b: ((c >> 8) & 0xff) as u8,
            a: (c & 0xff) as u8,
        }
    }

    /// Lossy pack to 2-bits-per-component `rrggbbaa`. Matches `to_u8`.
    #[inline]
    pub fn to_u8(self) -> u8 {
        ((self.r >> 6) << 6) | ((self.g >> 6) << 4) | ((self.b >> 6) << 2) | (self.a >> 6)
    }

    /// Lossy pack to 4-bits-per-component `rrrrggggbbbbaaaa`. Matches `to_u16`.
    #[inline]
    pub fn to_u16(self) -> u16 {
        (((self.r >> 4) as u16) << 12)
            | (((self.g >> 4) as u16) << 8)
            | (((self.b >> 4) as u16) << 4)
            | ((self.a >> 4) as u16)
    }

    /// Pack to `rrrrrrrrggggggggbbbbbbbbaaaaaaaa`. Matches `to_u32` / `packed_value`.
    #[inline]
    pub fn to_u32(self) -> u32 {
        ((self.r as u32) << 24) | ((self.g as u32) << 16) | ((self.b as u32) << 8) | (self.a as u32)
    }
}

/// Linear interpolation between two colors. Matches `color.h math::lerp`.
#[inline]
pub fn lerp(a: Color, b: Color, t: f32) -> Color {
    Color::new(
        funcs::lerp_f32(a.r, b.r, t),
        funcs::lerp_f32(a.g, b.g, t),
        funcs::lerp_f32(a.b, b.b, t),
        funcs::lerp_f32(a.a, b.a, t),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color8_u32_roundtrip() {
        let c = Color8::new(0x12, 0x34, 0x56, 0x78);
        assert_eq!(c.to_u32(), 0x12345678);
        assert_eq!(Color8::from_u32(0x12345678), c);
    }

    #[test]
    fn color8_u16_decode_encode() {
        // 4-bit-per-channel decode scales by 17, so re-encoding (>>4) is lossless.
        let c = Color8::from_u16(0x1234);
        assert_eq!(c, Color8::new(0x11, 0x22, 0x33, 0x44));
        assert_eq!(c.to_u16(), 0x1234);
    }

    #[test]
    fn color8_u8_decode() {
        // 2-bit-per-channel: 0b11_10_01_00 -> r=255,g=170,b=85,a=0 (each *85).
        let c = Color8::from_u8(0b11_10_01_00);
        assert_eq!(c, Color8::new(255, 170, 85, 0));
    }

    #[test]
    fn color_roundtrip_via_color8() {
        let c = Color::from_rgb(1.0, 0.5, 0.0);
        let back = Color8::from_color(c).to_color();
        assert!((back.r - 1.0).abs() < 0.01);
        // 0.5 -> 127 (truncated) -> 127/255 ≈ 0.498.
        assert!((back.g - 0.5).abs() < 0.01);
        assert!((back.b - 0.0).abs() < 0.01);
    }

    #[test]
    fn color_lerp() {
        let a = Color::new(0.0, 0.0, 0.0, 0.0);
        let b = Color::new(1.0, 1.0, 1.0, 1.0);
        let m = lerp(a, b, 0.5);
        assert!((m.r - 0.5).abs() < 1e-5 && (m.a - 0.5).abs() < 1e-5);
    }

    #[test]
    fn component_index() {
        let c = Color8::new(1, 2, 3, 4);
        assert_eq!(c.component(0), 1);
        assert_eq!(c.component(3), 4);
    }
}
