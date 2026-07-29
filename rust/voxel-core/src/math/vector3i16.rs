//! `Vector3i16` — compact 16-bit `Vector3T<i16>`.
//!
//! Ported from `util/math/vector3i16.h`. The type itself is just
//! `Vector3T<i16>` (operators come from the [`super::vector3`] macro); this
//! module adds the [`Vector3i16`] alias and the packing hash helper
//! `get_hash_st`, used to key compact voxel coordinates in hash maps.

use super::vector3::Vector3T;

/// 3D vector of signed 16-bit integers. Matches `Vector3T<int16_t>`.
pub type Vector3i16 = Vector3T<i16>;

impl Vector3i16 {
    /// Pack the three components into one `u64` and return its hash.
    ///
    /// Matches `get_hash_st`: `x` goes in bits 0..16, `y` in 16..32, `z` in
    /// 32..48; high 16 bits are unused (unless sign-extension from a negative
    /// `i16` contaminates them — that mirrors the C++ behaviour, which widens
    /// through `int` before the `|`, so negative components set the upper bits).
    /// The result is then hashed through the default integer hasher.
    ///
    /// The hash only needs to be deterministic; it is not required to be
    /// collision-free.
    #[inline]
    pub fn pack_hash(self) -> u64 {
        // Reproduce C++ `uint64_t(v.x)`: int16 -> int (sign-extend) -> uint64.
        let x = self.x as i32 as i64 as u64;
        let y = self.y as i32 as i64 as u64;
        let z = self.z as i32 as i64 as u64;
        x | (y << 16) | (z << 32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_and_ops() {
        let a = Vector3i16::new(1, 2, 3);
        let b = Vector3i16::new(4, 5, 6);
        assert_eq!(a + b, Vector3i16::new(5, 7, 9));
        assert_eq!(b - a, Vector3i16::new(3, 3, 3));
        assert_eq!(a * 2, Vector3i16::new(2, 4, 6));
    }

    #[test]
    fn pack_hash_deterministic_and_distinct() {
        let a = Vector3i16::new(1, 2, 3);
        let b = Vector3i16::new(3, 2, 1);
        assert_ne!(a.pack_hash(), b.pack_hash());
        // Same input → same hash.
        assert_eq!(a.pack_hash(), a.pack_hash());
    }

    #[test]
    fn pack_hash_nonnegative_layout() {
        // Positive components land in distinct 16-bit lanes.
        let h = Vector3i16::new(0x0001, 0x0002, 0x0003).pack_hash();
        assert_eq!(h & 0xFFFF, 0x0001);
        assert_eq!((h >> 16) & 0xFFFF, 0x0002);
        assert_eq!((h >> 32) & 0xFFFF, 0x0003);
    }

    #[test]
    fn pack_hash_negative_matches_cpp_sign_extension() {
        // C++ widens int16(-1) through int before the |, so the low lane and all
        // higher bits become set. Reproduced here: result is all-ones.
        let h = Vector3i16::new(-1, 0, 0).pack_hash();
        assert_eq!(h, u64::MAX);
    }
}
