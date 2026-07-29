//! Hash function helpers.
//!
//! Ported from `util/hash_funcs.h` (itself copied from Godot core). Provides
//! the `djb2`-variant and MurmurHash3 one-shot combiners used throughout the
//! engine to hash vectors and other aggregate keys. All are deterministic and
//! platform-independent (no SIMD / no endianness dependence for these inputs).

/// DJB2-style 32-bit hash combiner. Matches `hash_djb2_one_32`.
///
/// Chains by calling repeatedly with the previous result as `prev`:
/// `hash_djb2_one_32(y, hash_djb2_one_32(x))`. The default seed is `5381`.
#[inline]
pub fn hash_djb2_one_32(input: u32, prev: u32) -> u32 {
    ((prev << 5).wrapping_add(prev)) ^ input
}

/// One-shot DJB2 with the default seed (`5381`). Matches the
/// `hash_djb2_one_32(p_in)` single-argument overload.
#[inline]
pub fn hash_djb2_one_32_seeded(input: u32) -> u32 {
    hash_djb2_one_32(input, 5381)
}

/// DJB2-style 64-bit hash combiner. Matches `hash_djb2_one_64`.
#[inline]
pub fn hash_djb2_one_64(input: u64, prev: u64) -> u64 {
    ((prev << 5).wrapping_add(prev)) ^ input
}

/// Default MurmurHash3 seed (matches `HASH_MURMUR3_SEED`).
pub const HASH_MURMUR3_SEED: u32 = 0x7F07C65;

/// MurmurHash3 (32-bit) one-shot combiner. Matches `hash_murmur3_one_32`.
#[inline]
pub fn hash_murmur3_one_32(mut input: u32, mut seed: u32) -> u32 {
    input = input.wrapping_mul(0xcc9e2d51);
    input = input.rotate_left(15);
    input = input.wrapping_mul(0x1b873593);

    seed ^= input;
    seed = seed.rotate_left(13);
    seed = seed.wrapping_mul(5).wrapping_add(0xe6546b64);

    seed
}

/// MurmurHash3 finalizer `fmix32`. Matches `hash_fmix32`.
#[inline]
pub fn hash_fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn djb2_default_seed_is_5381() {
        // With prev == seed (5381), one-shot equals the chained form.
        assert_eq!(hash_djb2_one_32_seeded(0), hash_djb2_one_32(0, 5381));
    }

    #[test]
    fn djb2_chaining_is_order_dependent() {
        // hash(x then y) != hash(y then x): the combiner is not commutative.
        let x = hash_djb2_one_32(1, 5381);
        let y = hash_djb2_one_32(2, 5381);
        let xy = hash_djb2_one_32(2, x);
        let yx = hash_djb2_one_32(1, y);
        assert_ne!(xy, yx);
    }

    #[test]
    fn djb2_known_value() {
        // Hand-computed: prev=5381 -> (5381<<5)+5381 = 5381*33 = 177573; 177573 ^ 1 = 177572.
        assert_eq!(hash_djb2_one_32(1, 5381), 177572);
    }

    #[test]
    fn djb2_64_matches_32_pattern() {
        // Same arithmetic, 64-bit wide. prev=5381, input=1.
        assert_eq!(hash_djb2_one_64(1, 5381), 177572u64);
    }

    #[test]
    fn murmur3_is_deterministic() {
        let a = hash_murmur3_one_32(42, HASH_MURMUR3_SEED);
        let b = hash_murmur3_one_32(42, HASH_MURMUR3_SEED);
        assert_eq!(a, b);
        // Different inputs (very likely) produce different hashes.
        assert_ne!(
            hash_murmur3_one_32(42, HASH_MURMUR3_SEED),
            hash_murmur3_one_32(43, HASH_MURMUR3_SEED)
        );
    }

    #[test]
    fn fmix32_identity_and_distribution() {
        // fmix is a bijection on u32, so fmix(0) is just some constant.
        let _ = hash_fmix32(0);
        // Two different inputs should (with overwhelming probability) differ.
        assert_ne!(hash_fmix32(1), hash_fmix32(2));
    }
}
