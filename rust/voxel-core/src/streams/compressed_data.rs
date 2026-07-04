//! `streams::compressed_data` — LZ4/ZSTD compression envelope.
//!
//! Ported from `streams/compressed_data.{h,cpp}`. On-disk layout:
//!
//! ```text
//! [u8 compression_tag][optional u32 decompressed_size][compressed bytes...]
//! ```
//!
//! The tag selects one of:
//! - [`Compression::None`] — bytes follow verbatim (no size header).
//! - [`Compression::Lz4Be`] — **legacy** big-endian: `u32` size (BE) + LZ4 block.
//! - [`Compression::Lz4`] — current: `u32` size (LE) + LZ4 block.
//! - [`Compression::Zstd`] — `u32` size (LE) + ZSTD frame (only available with
//!   the `zstd` cargo feature; otherwise returns [`Error::Unsupported]).
//!
//! LZ4 is provided by [`lz4_flex`] (pure Rust, no C/FFI) so the default build
//! stays cross-compilation-friendly for Android/WASM. ZSTD lives behind a
//! feature because the `zstd` crate bundles a C library.

use crate::io::serialization::{Endianness, MemoryReader, MemoryWriter};

/// Size of the on-disk header when a `u32` decompressed-size prefix is present
/// (one tag byte + four size bytes). Matches the C++ `header_size` constant.
const SIZE_HEADER_LEN: usize = 1 + 4;

/// Compression format selector. Ported from `CompressedData::Compression`.
///
/// The discriminant values are a wire-format contract (written as the leading
/// tag byte) — do not renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    /// No compression. Bytes follow verbatim. No size header.
    None = 0,
    /// **Legacy** LZ4 with a big-endian size prefix. Deprecated; kept only for
    /// reading old saves.
    Lz4Be = 1,
    /// LZ4 block with a little-endian size prefix. Current default.
    Lz4 = 2,
    /// ZSTD frame. Requires the `zstd` cargo feature.
    Zstd = 3,
}

impl Compression {
    /// Number of valid tags. Matches `COMPRESSION_COUNT`.
    pub const COUNT: u8 = 4;

    /// Parse a tag byte, returning `None` if it isn't a known compression.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Lz4Be),
            2 => Some(Self::Lz4),
            3 => Some(Self::Zstd),
            _ => None,
        }
    }
}

/// Why (de)compression failed. Mirrors the `false`-return paths in the C++.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Truncated input (missing tag, size header, or payload).
    UnexpectedEof,
    /// Tag byte was outside `0..Compression::COUNT`.
    InvalidCompression(u8),
    /// Decoded size prefix was negative or implausibly large.
    InvalidSize(i64),
    /// LZ4 reported the stream is corrupt or the decoded length mismatched.
    Lz4(String),
    /// ZSTD support is not compiled in (enable the `zstd` feature).
    Unsupported,
    /// ZSTD encode/decode returned an error.
    #[cfg(feature = "zstd")]
    Zstd(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnexpectedEof => write!(f, "compressed_data: unexpected end of input"),
            Error::InvalidCompression(v) => {
                write!(f, "compressed_data: invalid compression tag {v}")
            }
            Error::InvalidSize(s) => write!(f, "compressed_data: invalid size {s}"),
            Error::Lz4(m) => write!(f, "compressed_data: lz4 error: {m}"),
            Error::Unsupported => write!(f, "compressed_data: zstd not compiled in"),
            #[cfg(feature = "zstd")]
            Error::Zstd(m) => write!(f, "compressed_data: zstd error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Decompression
// ---------------------------------------------------------------------------

/// Decompress `src` into `dst`, clearing it first. Ported from
/// `CompressedData::decompress`.
pub fn decompress(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    let mut r = MemoryReader::little(src);
    let tag = r.try_get_8().ok_or(Error::UnexpectedEof)?;
    let comp = Compression::from_u8(tag).ok_or(Error::InvalidCompression(tag))?;

    match comp {
        Compression::None => {
            // No size header; the rest of `src` is the payload verbatim.
            dst.clear();
            dst.extend_from_slice(&src[1..]);
            Ok(())
        }
        Compression::Lz4Be => {
            // Legacy path: switch the reader to big-endian for the size prefix.
            r.set_endianness(Endianness::BigEndian);
            decompress_lz4(&mut r, src, dst)
        }
        Compression::Lz4 => decompress_lz4(&mut r, src, dst),
        Compression::Zstd => decompress_zstd(&mut r, src, dst),
    }
}

/// Shared LZ4 path for both [`Compression::Lz4`] and [`Compression::Lz4Be`];
/// the reader's endianness is set by the caller. Matches `decompress_lz4`.
fn decompress_lz4(r: &mut MemoryReader<'_>, src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    let decompressed_size = i64::from(r.try_get_32().ok_or(Error::UnexpectedEof)?);
    if decompressed_size < 0 {
        return Err(Error::InvalidSize(decompressed_size));
    }
    let decompressed_size = decompressed_size as usize;

    // Compressed payload starts right after the tag+size header.
    let payload = src.get(SIZE_HEADER_LEN..).ok_or(Error::UnexpectedEof)?;

    dst.resize(decompressed_size, 0);
    // `decompress_into` mirrors `LZ4_decompress_safe`: it refuses to write past
    // `dst.len()` and errors out if `payload` is malformed.
    let written =
        lz4_flex::block::decompress_into(payload, dst).map_err(|e| Error::Lz4(e.to_string()))?;
    if written != decompressed_size {
        return Err(Error::Lz4(format!(
            "expected {decompressed_size} bytes, got {written}"
        )));
    }
    dst.truncate(written);
    Ok(())
}

#[cfg(feature = "zstd")]
fn decompress_zstd(r: &mut MemoryReader<'_>, src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    let decompressed_size = i64::from(r.try_get_32().ok_or(Error::UnexpectedEof)?);
    if decompressed_size < 0 {
        return Err(Error::InvalidSize(decompressed_size));
    }
    let decompressed_size = decompressed_size as usize;

    let payload = src.get(SIZE_HEADER_LEN..).ok_or(Error::UnexpectedEof)?;
    dst.clear();
    dst.resize(decompressed_size, 0);
    let written = zstd::stream::decode_all(payload).map_err(|e| Error::Zstd(e.to_string()))?;
    if written.len() != decompressed_size {
        return Err(Error::Zstd(format!(
            "expected {decompressed_size} bytes, got {}",
            written.len()
        )));
    }
    dst.copy_from_slice(&written);
    Ok(())
}

#[cfg(not(feature = "zstd"))]
fn decompress_zstd(_r: &mut MemoryReader<'_>, _src: &[u8], _dst: &mut Vec<u8>) -> Result<()> {
    Err(Error::Unsupported)
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

/// Compress `src` into `dst` using `comp`, clearing `dst` first. Ported from
/// `CompressedData::compress`.
pub fn compress(src: &[u8], dst: &mut Vec<u8>, comp: Compression) -> Result<()> {
    match comp {
        Compression::None => {
            dst.clear();
            dst.push(Compression::None as u8);
            dst.extend_from_slice(src);
            Ok(())
        }
        Compression::Lz4Be => compress_lz4(src, dst, Endianness::BigEndian, Compression::Lz4Be),
        Compression::Lz4 => compress_lz4(src, dst, Endianness::LittleEndian, Compression::Lz4),
        Compression::Zstd => compress_zstd(src, dst),
    }
}

/// LZ4 block path. Writes the tag byte, the (endian-specific) decompressed-size
/// prefix, then the LZ4-compressed payload. Matches `compress_lz4`. The
/// `tag_for` parameter is the compression variant we're emitting (Lz4 or
/// Lz4Be) — kept explicit so the wire format stays unambiguous.
fn compress_lz4(
    src: &[u8],
    dst: &mut Vec<u8>,
    endianness: Endianness,
    tag_for: Compression,
) -> Result<()> {
    if src.len() > u32::MAX as usize {
        return Err(Error::InvalidSize(src.len() as i64));
    }
    dst.clear();
    {
        // Scope the writer so we can mutably borrow `dst` again afterwards.
        let mut w = MemoryWriter::new(dst, endianness);
        w.store_8(tag_for as u8);
        w.store_32(src.len() as u32);
    }
    let compressed = lz4_flex::block::compress(src);
    if compressed.is_empty() && !src.is_empty() {
        return Err(Error::Lz4(
            "compress returned empty for non-empty input".into(),
        ));
    }
    dst.extend_from_slice(&compressed);
    Ok(())
}

#[cfg(feature = "zstd")]
fn compress_zstd(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    dst.clear();
    {
        let mut w = MemoryWriter::little(dst);
        w.store_8(Compression::Zstd as u8);
        w.store_32(src.len() as u32);
    }
    let compressed = zstd::stream::encode_all(src, 0).map_err(|e| Error::Zstd(e.to_string()))?;
    if compressed.is_empty() {
        return Err(Error::Zstd("encode returned empty".into()));
    }
    dst.extend_from_slice(&compressed);
    Ok(())
}

#[cfg(not(feature = "zstd"))]
fn compress_zstd(_src: &[u8], _dst: &mut Vec<u8>) -> Result<()> {
    Err(Error::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Highly compressible sample: a long run of a single byte plus some noise.
    fn sample() -> Vec<u8> {
        let mut v = vec![0x41u8; 4096];
        // A sprinkling of distinct bytes keeps LZ4 from degenerating to a single
        // match (which would make the "round-trip differs from input" check
        // trivially true without exercising the decoder).
        for (i, b) in v.iter_mut().enumerate().step_by(257) {
            *b = (i as u8).wrapping_mul(7);
        }
        v
    }

    fn randomish_sample() -> Vec<u8> {
        // Pseudo-random but deterministic payload that LZ4 won't compress well;
        // ensures the round-trip exercises an actual back-and-forth.
        let mut v = Vec::with_capacity(1024);
        let mut x: u32 = 0xdeadbeef;
        for _ in 0..1024 {
            // xorshift32
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            v.push(x as u8);
        }
        v
    }

    #[test]
    fn none_round_trips_verbatim() {
        let src = sample();
        let mut dst = Vec::new();
        compress(&src, &mut dst, Compression::None).unwrap();
        // Tag byte + payload.
        assert_eq!(dst.len(), src.len() + 1);
        assert_eq!(dst[0], Compression::None as u8);

        let mut out = Vec::new();
        decompress(&dst, &mut out).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn lz4_round_trips_compressive_payload() {
        let src = sample();
        let mut dst = Vec::new();
        compress(&src, &mut dst, Compression::Lz4).unwrap();
        // Tag (1) + size (4) + compressed payload. LZ4 should beat the original
        // on a 4 KiB run-heavy payload.
        assert!(
            dst.len() < src.len() + SIZE_HEADER_LEN,
            "lz4 didn't compress"
        );
        assert_eq!(dst[0], Compression::Lz4 as u8);

        let mut out = Vec::new();
        decompress(&dst, &mut out).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn lz4_round_trips_incompressible_payload() {
        let src = randomish_sample();
        let mut dst = Vec::new();
        compress(&src, &mut dst, Compression::Lz4).unwrap();

        let mut out = Vec::new();
        decompress(&dst, &mut out).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn lz4_be_legacy_round_trips() {
        // The deprecated big-endian variant must still round-trip for parity.
        let src = sample();
        let mut dst = Vec::new();
        compress(&src, &mut dst, Compression::Lz4Be).unwrap();
        assert_eq!(dst[0], Compression::Lz4Be as u8);

        let mut out = Vec::new();
        decompress(&dst, &mut out).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn lz4_be_uses_big_endian_size_header() {
        // Distinguish LZ4_BE from LZ4 by inspecting the size header byte order.
        let src = vec![0u8; 10];
        let mut be = Vec::new();
        compress(&src, &mut be, Compression::Lz4Be).unwrap();
        let mut le = Vec::new();
        compress(&src, &mut le, Compression::Lz4).unwrap();

        // dst[1..5] is the decompressed-size u32 for 10 = 0x0000000a.
        // Big-endian packs it as [00,00,00,0a] → least-significant byte at [4].
        // Little-endian packs it as [0a,00,00,00] → LSB at [1].
        assert_eq!(
            be[1..5],
            [0, 0, 0, 0x0a],
            "LZ4_BE size header should be big-endian"
        );
        assert_eq!(
            le[1..5],
            [0x0a, 0, 0, 0],
            "LZ4 size header should be little-endian"
        );
    }

    #[test]
    fn empty_payload_round_trips_under_lz4() {
        let src: Vec<u8> = Vec::new();
        let mut dst = Vec::new();
        compress(&src, &mut dst, Compression::Lz4).unwrap();
        let mut out = Vec::new();
        decompress(&dst, &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn decompress_rejects_truncated_tag() {
        let mut out = Vec::new();
        assert_eq!(decompress(&[], &mut out), Err(Error::UnexpectedEof));
    }

    #[test]
    fn decompress_rejects_invalid_tag() {
        let mut out = Vec::new();
        // Tag byte 99 is not a valid compression.
        assert_eq!(
            decompress(&[99u8, 0, 0, 0, 0], &mut out),
            Err(Error::InvalidCompression(99))
        );
    }

    #[test]
    fn decompress_rejects_truncated_size_header() {
        let mut out = Vec::new();
        // LZ4 tag but no size bytes following.
        assert_eq!(
            decompress(&[Compression::Lz4 as u8], &mut out),
            Err(Error::UnexpectedEof)
        );
    }

    #[test]
    fn decompress_rejects_corrupt_lz4_payload() {
        // Build a valid header claiming 4096 decompressed bytes, then feed
        // garbage as the payload.
        let mut bad = vec![Compression::Lz4 as u8];
        bad.extend_from_slice(&4096u32.to_le_bytes());
        bad.extend_from_slice(&[0xff; 64]); // not a valid LZ4 stream
        let mut out = Vec::new();
        match decompress(&bad, &mut out) {
            Err(Error::Lz4(_)) => {}
            other => panic!("expected Lz4 error, got {other:?}"),
        }
    }

    #[test]
    fn compression_from_u8_round_trips_all_tags() {
        for v in 0..Compression::COUNT {
            let c = Compression::from_u8(v).expect("valid tag");
            assert_eq!(c as u8, v);
        }
        assert!(Compression::from_u8(Compression::COUNT).is_none());
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn zstd_round_trips_when_feature_enabled() {
        let src = sample();
        let mut dst = Vec::new();
        compress(&src, &mut dst, Compression::Zstd).unwrap();
        assert_eq!(dst[0], Compression::Zstd as u8);

        let mut out = Vec::new();
        decompress(&dst, &mut out).unwrap();
        assert_eq!(out, src);
    }

    #[cfg(not(feature = "zstd"))]
    #[test]
    fn zstd_returns_unsupported_without_feature() {
        let src = sample();
        let mut dst = Vec::new();
        assert_eq!(
            compress(&src, &mut dst, Compression::Zstd),
            Err(Error::Unsupported)
        );

        // A synthetic ZSTD-tagged stream must also report unsupported.
        let mut zstd_stream = vec![Compression::Zstd as u8];
        zstd_stream.extend_from_slice(&10u32.to_le_bytes());
        let mut out = Vec::new();
        assert_eq!(decompress(&zstd_stream, &mut out), Err(Error::Unsupported));
    }
}
