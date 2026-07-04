//! `ColorPalette` — 256-entry voxel color lookup table.
//!
//! Ported from `meshers/cubes/voxel_color_palette.{h,cpp}`. Maps small voxel
//! ids to RGBA colors so colored voxels can be stored as one byte (a palette
//! index) instead of four. The Godot `Resource`/`PackedColorArray`/
//! `PackedInt32Array` binding glue is omitted; the palette is plain data here.

use crate::math::{Color, Color8};

/// Number of entries. Matches `VoxelColorPalette::MAX_COLORS`.
pub const PALETTE_SIZE: usize = 256;

/// A 256-entry color palette. Ported from `VoxelColorPalette`.
///
/// Default: index 0 transparent, index 1 white, indices 2.. opaque black —
/// matching the C++ constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorPalette {
    colors: [Color8; PALETTE_SIZE],
}

impl Default for ColorPalette {
    fn default() -> Self {
        let mut colors = [Color8::new(0, 0, 0, 0); PALETTE_SIZE];
        // Index 0 stays transparent (the constructor's first assignment).
        colors[0] = Color8::new(0, 0, 0, 0);
        colors[1] = Color8::new(255, 255, 255, 255);
        for c in &mut colors[2..] {
            *c = Color8::new(0, 0, 0, 255);
        }
        Self { colors }
    }
}

impl ColorPalette {
    /// `set_color8` — raw 8-bit color setter. Bounds-checked in debug.
    #[inline]
    pub fn set_color8(&mut self, index: u8, color: Color8) {
        self.colors[index as usize] = color;
    }

    /// `get_color8` — raw 8-bit color getter.
    #[inline]
    pub fn get_color8(&self, index: u8) -> Color8 {
        self.colors[index as usize]
    }

    /// `set_color` — float-color setter. Matches `set_color(int, Color)`.
    pub fn set_color(&mut self, index: usize, color: Color) {
        debug_assert!(index < PALETTE_SIZE);
        self.colors[index] = Color8::from_color(color);
    }

    /// `get_color` — float-color getter. Matches `get_color(int)`.
    pub fn get_color(&self, index: usize) -> Color {
        debug_assert!(index < PALETTE_SIZE);
        self.colors[index].to_color()
    }

    /// `clear` — reset all entries to the default `Color8` (transparent black).
    pub fn clear(&mut self) {
        for c in &mut self.colors {
            *c = Color8::new(0, 0, 0, 0);
        }
    }

    /// Whole-palette accessor (for serialization / bulk copy).
    pub fn colors(&self) -> &[Color8; PALETTE_SIZE] {
        &self.colors
    }

    /// Mutable whole-palette accessor.
    pub fn colors_mut(&mut self) -> &mut [Color8; PALETTE_SIZE] {
        &mut self.colors
    }

    /// Serialize as packed `u32` values (`0xRRGGBBAA`), matching
    /// `_b_get_data` / `Color8::to_u32`. Useful for the on-disk region format.
    pub fn to_u32_array(&self) -> [u32; PALETTE_SIZE] {
        let mut out = [0u32; PALETTE_SIZE];
        for (i, c) in self.colors.iter().enumerate() {
            out[i] = c.to_u32();
        }
        out
    }

    /// Deserialize from packed `u32` values, matching `_b_set_data`. Entries
    /// beyond `data.len()` are left untouched (matching the C++ behavior).
    pub fn set_from_u32_array(&mut self, data: &[u32]) {
        let n = data.len().min(PALETTE_SIZE);
        for (i, &v) in data[..n].iter().enumerate() {
            self.colors[i] = Color8::from_u32(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_has_transparent_white_black_pattern() {
        let p = ColorPalette::default();
        assert_eq!(p.get_color8(0), Color8::new(0, 0, 0, 0));
        assert_eq!(p.get_color8(1), Color8::new(255, 255, 255, 255));
        assert_eq!(p.get_color8(2), Color8::new(0, 0, 0, 255));
        assert_eq!(p.get_color8(255), Color8::new(0, 0, 0, 255));
    }

    #[test]
    fn set_get_color8_round_trips() {
        let mut p = ColorPalette::default();
        p.set_color8(42, Color8::new(10, 20, 30, 40));
        assert_eq!(p.get_color8(42), Color8::new(10, 20, 30, 40));
    }

    #[test]
    fn set_get_color_uses_float_conversion() {
        let mut p = ColorPalette::default();
        p.set_color(5, Color::new(1.0, 0.0, 0.0, 1.0));
        let c = p.get_color(5);
        assert!((c.r - 1.0).abs() < 1e-3 && (c.g - 0.0).abs() < 1e-3);
    }

    #[test]
    fn clear_resets_all_entries() {
        let mut p = ColorPalette::default();
        p.set_color8(10, Color8::new(255, 255, 255, 255));
        p.clear();
        assert_eq!(p.get_color8(10), Color8::new(0, 0, 0, 0));
    }

    #[test]
    fn u32_round_trip_preserves_colors() {
        let mut p = ColorPalette::default();
        p.set_color8(0, Color8::new(0xff, 0x00, 0xab, 0xcd));
        p.set_color8(255, Color8::new(0x01, 0x02, 0x03, 0x04));
        let packed = p.to_u32_array();
        let mut p2 = ColorPalette::default();
        p2.set_from_u32_array(&packed);
        assert_eq!(p, p2);
    }

    #[test]
    fn set_from_u32_array_handles_short_input() {
        let mut p = ColorPalette::default();
        // Only set the first 3 entries; the rest keep their previous values.
        p.set_from_u32_array(&[0x11223344, 0x55667788]);
        assert_eq!(p.get_color8(0), Color8::from_u32(0x11223344));
        assert_eq!(p.get_color8(1), Color8::from_u32(0x55667788));
        // Index 2 untouched from default.
        assert_eq!(p.get_color8(2), Color8::new(0, 0, 0, 255));
    }
}
