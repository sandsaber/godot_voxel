//! `constants::cube_tables` — cube geometry lookup tables.
//!
//! Ported from `constants/cube_tables.{h,cpp}`. Provides the `Side`, `Edge`,
//! `Corner` enums and their associated lookup tables (normals, corner
//! positions, side→corner/edge maps, edge→corner pairs, Moore neighborhood).
//! These are consumed by the blocky mesher, the cubes mesher, and any
//! neighbor-aware voxel algorithm.
//!
//! # Convention
//! ```text
//!    7-------6
//!   /|      /|
//!  / |     / |  Corners
//! 4-------5  |
//! |  3----|--2
//! | /     | /     y z
//! |/      |/      |/   OpenGL axis convention
//! 0-------1    x--o
//! ```

use crate::math::{Vector3f, Vector3i};

/// The six faces of a cube. Matches `Cube::Side`. The discriminant values are
/// a wire-format contract used in masks and baked-model data — do not
/// renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Side {
    /// `+X`
    Left = 0,
    /// `-X`
    Right = 1,
    /// `-Y`
    Bottom = 2,
    /// `+Y`
    Top = 3,
    /// `-Z`
    Back = 4,
    /// `+Z`
    Front = 5,
}

impl Side {
    pub const COUNT: usize = 6;

    /// Convert a discriminant back to a `Side`. Returns `None` if out of range.
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Left),
            1 => Some(Self::Right),
            2 => Some(Self::Bottom),
            3 => Some(Self::Top),
            4 => Some(Self::Back),
            5 => Some(Self::Front),
            _ => None,
        }
    }
}

/// Axis-aligned direction, used by some tables. Matches `Cube::SideAxis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SideAxis {
    PositiveX = 0,
    NegativeX = 1,
    NegativeY = 2,
    PositiveY = 3,
    NegativeZ = 4,
    PositiveZ = 5,
}

/// The 12 edges of a cube. Matches `Cube::Edge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Edge {
    BottomBack = 0,
    BottomRight = 1,
    BottomFront = 2,
    BottomLeft = 3,
    BackLeft = 4,
    BackRight = 5,
    FrontRight = 6,
    FrontLeft = 7,
    TopBack = 8,
    TopRight = 9,
    TopFront = 10,
    TopLeft = 11,
}

impl Edge {
    pub const COUNT: usize = 12;
}

/// The 8 corners of a cube. Matches `Cube::Corner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Corner {
    BottomBackLeft = 0,
    BottomBackRight = 1,
    BottomFrontRight = 2,
    BottomFrontLeft = 3,
    TopBackLeft = 4,
    TopBackRight = 5,
    TopFrontRight = 6,
    TopFrontLeft = 7,
}

impl Corner {
    pub const COUNT: usize = 8;
}

// -----------------------------------------------------------------------
// Lookup tables — verbatim from cube_tables.cpp
// -----------------------------------------------------------------------

/// Unit position of each corner within a 0..1 cube. Matches `g_corner_position`.
pub const CORNER_POSITION: [Vector3f; Corner::COUNT] = [
    Vector3f::new(1.0, 0.0, 0.0),
    Vector3f::new(0.0, 0.0, 0.0),
    Vector3f::new(0.0, 0.0, 1.0),
    Vector3f::new(1.0, 0.0, 1.0),
    Vector3f::new(1.0, 1.0, 0.0),
    Vector3f::new(0.0, 1.0, 0.0),
    Vector3f::new(0.0, 1.0, 1.0),
    Vector3f::new(1.0, 1.0, 1.0),
];

/// Triangle indices for each side's quad (two triangles, 6 indices). Matches
/// `g_side_quad_triangles`.
pub const SIDE_QUAD_TRIANGLES: [[i32; 6]; Side::COUNT] = [
    [0, 2, 1, 0, 3, 2], // Left
    [0, 2, 1, 0, 3, 2], // Right
    [0, 2, 1, 0, 3, 2], // Bottom
    [0, 2, 1, 0, 3, 2], // Top
    [0, 2, 1, 0, 3, 2], // Back
    [0, 2, 1, 0, 3, 2], // Front
];

/// Outward normal of each side. Matches `g_side_normals`.
pub const SIDE_NORMALS: [Vector3i; Side::COUNT] = [
    Vector3i::new(1, 0, 0),  // Left
    Vector3i::new(-1, 0, 0), // Right
    Vector3i::new(0, -1, 0), // Bottom
    Vector3i::new(0, 1, 0),  // Top
    Vector3i::new(0, 0, -1), // Back
    Vector3i::new(0, 0, 1),  // Front
];

/// Tangent of each side (xyz + sign). Matches `g_side_tangents`.
pub const SIDE_TANGENTS: [[f32; 4]; Side::COUNT] = [
    [0.0, 0.0, -1.0, 1.0],
    [0.0, 0.0, 1.0, 1.0],
    [1.0, 0.0, 0.0, 1.0],
    [-1.0, 0.0, 0.0, 1.0],
    [-1.0, 0.0, 0.0, 1.0],
    [1.0, 0.0, 0.0, 1.0],
];

/// For each side, the four corners (by index) that bound it. Matches
/// `g_side_corners`.
pub const SIDE_CORNERS: [[usize; 4]; Side::COUNT] = [
    [3, 0, 4, 7], // Left
    [1, 2, 6, 5], // Right
    [1, 0, 3, 2], // Bottom
    [4, 5, 6, 7], // Top
    [0, 1, 5, 4], // Back
    [2, 3, 7, 6], // Front
];

/// For each side, the four edges (by index) that bound it. Matches
/// `g_side_edges`.
pub const SIDE_EDGES: [[usize; 4]; Side::COUNT] = [
    [3, 7, 11, 4],
    [1, 6, 9, 5],
    [0, 1, 2, 3],
    [8, 9, 10, 11],
    [0, 5, 8, 4],
    [2, 6, 10, 7],
];

/// Integer normal direction for each corner (the sum of the three side
/// normals meeting at that corner). Matches `g_corner_inormals`.
pub const CORNER_INORMALS: [Vector3i; Corner::COUNT] = [
    Vector3i::new(1, -1, -1),
    Vector3i::new(-1, -1, -1),
    Vector3i::new(-1, -1, 1),
    Vector3i::new(1, -1, 1),
    Vector3i::new(1, 1, -1),
    Vector3i::new(-1, 1, -1),
    Vector3i::new(-1, 1, 1),
    Vector3i::new(1, 1, 1),
];

/// Integer normal direction for each edge. Matches `g_edge_inormals`.
pub const EDGE_INORMALS: [Vector3i; Edge::COUNT] = [
    Vector3i::new(0, -1, -1),
    Vector3i::new(-1, -1, 0),
    Vector3i::new(0, -1, 1),
    Vector3i::new(1, -1, 0),
    Vector3i::new(1, 0, -1),
    Vector3i::new(-1, 0, -1),
    Vector3i::new(-1, 0, 1),
    Vector3i::new(1, 0, 1),
    Vector3i::new(0, 1, -1),
    Vector3i::new(-1, 1, 0),
    Vector3i::new(0, 1, 1),
    Vector3i::new(1, 1, 0),
];

/// For each edge, the two corners it connects. Matches `g_edge_corners`.
pub const EDGE_CORNERS: [[usize; 2]; Edge::COUNT] = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
];

/// Opposite side for each side. Matches `g_opposite_side`.
/// Indexed by `Side as usize`; returns the `Side` discriminant that faces the
/// other way.
pub const OPPOSITE_SIDE: [u8; Side::COUNT] = [
    Side::Right as u8,
    Side::Left as u8,
    Side::Top as u8,
    Side::Bottom as u8,
    Side::Front as u8,
    Side::Back as u8,
];

/// Number of cells in the 3×3×3 Moore neighborhood (excluding the center).
pub const MOORE_NEIGHBORING_3D_COUNT: usize = 26;
/// Number of cells in the full 3×3×3 Moore area (including the center).
pub const MOORE_AREA_3D_COUNT: usize = 27;
/// Index of the central cell in [`ORDERED_MOORE_AREA_3D`].
pub const MOORE_AREA_3D_CENTRAL_INDEX: usize = 13;

/// The 26 Moore neighbors (3×3×3 minus center). Order is not significant.
/// Matches `g_moore_neighboring_3d`.
pub const MOORE_NEIGHBORING_3D: [Vector3i; MOORE_NEIGHBORING_3D_COUNT] = [
    Vector3i::new(-1, -1, -1),
    Vector3i::new(0, -1, -1),
    Vector3i::new(1, -1, -1),
    Vector3i::new(-1, -1, 0),
    Vector3i::new(0, -1, 0),
    Vector3i::new(1, -1, 0),
    Vector3i::new(-1, -1, 1),
    Vector3i::new(0, -1, 1),
    Vector3i::new(1, -1, 1),
    Vector3i::new(-1, 0, -1),
    Vector3i::new(0, 0, -1),
    Vector3i::new(1, 0, -1),
    Vector3i::new(-1, 0, 0),
    Vector3i::new(1, 0, 0),
    Vector3i::new(-1, 0, 1),
    Vector3i::new(0, 0, 1),
    Vector3i::new(1, 0, 1),
    Vector3i::new(-1, 1, -1),
    Vector3i::new(0, 1, -1),
    Vector3i::new(1, 1, -1),
    Vector3i::new(-1, 1, 0),
    Vector3i::new(0, 1, 0),
    Vector3i::new(1, 1, 0),
    Vector3i::new(-1, 1, 1),
    Vector3i::new(0, 1, 1),
    Vector3i::new(1, 1, 1),
];

/// The full 3×3×3 Moore area in XYZ iteration order (center at index 13).
/// Matches `g_ordered_moore_area_3d`. Order matters for lock-free multithreaded
/// iteration (avoids deadlock by establishing a canonical acquisition order).
pub const ORDERED_MOORE_AREA_3D: [Vector3i; MOORE_AREA_3D_COUNT] = [
    Vector3i::new(-1, -1, -1),
    Vector3i::new(0, -1, -1),
    Vector3i::new(1, -1, -1),
    Vector3i::new(-1, 0, -1),
    Vector3i::new(0, 0, -1),
    Vector3i::new(1, 0, -1),
    Vector3i::new(-1, 1, -1),
    Vector3i::new(0, 1, -1),
    Vector3i::new(1, 1, -1),
    Vector3i::new(-1, -1, 0),
    Vector3i::new(0, -1, 0),
    Vector3i::new(1, -1, 0),
    Vector3i::new(-1, 0, 0),
    Vector3i::new(0, 0, 0),
    Vector3i::new(1, 0, 0),
    Vector3i::new(-1, 1, 0),
    Vector3i::new(0, 1, 0),
    Vector3i::new(1, 1, 0),
    Vector3i::new(-1, -1, 1),
    Vector3i::new(0, -1, 1),
    Vector3i::new(1, -1, 1),
    Vector3i::new(-1, 0, 1),
    Vector3i::new(0, 0, 1),
    Vector3i::new(1, 0, 1),
    Vector3i::new(-1, 1, 1),
    Vector3i::new(0, 1, 1),
    Vector3i::new(1, 1, 1),
];

/// Map a direction vector to the [`Side`] whose normal it matches. Matches
/// `dir_to_side`. Returns `Side::Front` (the C++ fallback) if no side matches.
pub fn dir_to_side(d: Vector3i) -> Side {
    for (i, n) in SIDE_NORMALS.iter().enumerate() {
        if *n == d {
            return Side::from_u8(i as u8).unwrap();
        }
    }
    // C++ logs an error and returns FRONT; we do the same for parity.
    Side::Front
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_count_is_six() {
        assert_eq!(Side::COUNT, 6);
    }

    #[test]
    fn side_from_u8_round_trips() {
        for i in 0..Side::COUNT as u8 {
            let s = Side::from_u8(i).expect("valid side");
            assert_eq!(s as u8, i);
        }
        assert!(Side::from_u8(Side::COUNT as u8).is_none());
    }

    #[test]
    fn side_normals_are_unit_and_opposite_pairs() {
        // Left/Right, Bottom/Top, Back/Front.
        for &(a, b) in &[
            (Side::Left, Side::Right),
            (Side::Bottom, Side::Top),
            (Side::Back, Side::Front),
        ] {
            let na = SIDE_NORMALS[a as usize];
            let nb = SIDE_NORMALS[b as usize];
            assert_eq!(na.x + nb.x, 0);
            assert_eq!(na.y + nb.y, 0);
            assert_eq!(na.z + nb.z, 0);
        }
    }

    #[test]
    fn opposite_side_is_involution() {
        // Applying opposite twice returns the original side.
        for i in 0..Side::COUNT as u8 {
            let s = Side::from_u8(i).unwrap();
            let opp = OPPOSITE_SIDE[s as usize];
            let opp_back = OPPOSITE_SIDE[opp as usize];
            assert_eq!(opp_back, i);
        }
    }

    #[test]
    fn dir_to_side_finds_all_normals() {
        for (i, n) in SIDE_NORMALS.iter().enumerate() {
            assert_eq!(dir_to_side(*n) as usize, i);
        }
    }

    #[test]
    fn side_quad_triangles_have_six_indices() {
        for tri in &SIDE_QUAD_TRIANGLES {
            assert_eq!(tri.len(), 6);
        }
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn corner_positions_are_distinct() {
        // f32 doesn't impl Eq/Hash, so compare pairwise instead of a HashSet.
        for i in 0..Corner::COUNT {
            for j in (i + 1)..Corner::COUNT {
                assert_ne!(CORNER_POSITION[i], CORNER_POSITION[j], "dup corner {i}/{j}");
            }
        }
    }

    #[test]
    fn moore_neighborhood_excludes_center() {
        assert_eq!(MOORE_NEIGHBORING_3D_COUNT, 27 - 1);
        assert!(!MOORE_NEIGHBORING_3D.contains(&Vector3i::new(0, 0, 0)));
    }

    #[test]
    fn ordered_moore_area_has_center_at_13() {
        assert_eq!(
            ORDERED_MOORE_AREA_3D[MOORE_AREA_3D_CENTRAL_INDEX],
            Vector3i::new(0, 0, 0)
        );
        assert_eq!(ORDERED_MOORE_AREA_3D.len(), 27);
    }

    #[test]
    fn edge_corners_are_valid_pairs() {
        for pair in &EDGE_CORNERS {
            assert_eq!(pair.len(), 2);
            assert_ne!(pair[0], pair[1]);
            assert!(pair[0] < Corner::COUNT);
            assert!(pair[1] < Corner::COUNT);
        }
    }

    #[test]
    fn side_corners_cover_four_distinct() {
        for corners in &SIDE_CORNERS {
            use std::collections::HashSet;
            let set: HashSet<usize> = corners.iter().copied().collect();
            assert_eq!(set.len(), 4);
        }
    }
}
