//! `streams::block_serializer` — `VoxelBuffer` ↔ bytes.
//!
//! Ported from `streams/voxel_block_serializer.{h,cpp}`. Serializes a
//! [`VoxelBuffer`] into a compact on-disk byte stream (version 4), with optional
//! LZ4/ZSTD compression via [`crate::streams::compressed_data`].
//!
//! # Wire format (version 4)
//! ```text
//! u8  format_version (= 4)
//! u16 size.x   u16 size.y   u16 size.z
//! for each of 8 channels:
//!     u8  fmt  // low nibble: compression, high nibble: depth
//!     // Compression::None    -> raw voxel bytes (volume * depth_bytes)
//!     // Compression::Uniform -> single raw voxel (depth_bytes)
//! [u32 metadata_size + metadata bytes]   // OMITTED in this port (see below)
//! u32 trailing_magic (= 0x900df00d)
//! ```
//!
//! # Metadata: deferred
//! The C++ serializer writes a per-block + per-voxel metadata section keyed on
//! Godot `Variant` / a custom-metadata factory (`storage/metadata/`). That
//! subsystem is not yet ported, so this module emits and accepts the v4 format
//! **without** a metadata section — which is byte-compatible with the C++
//! behaviour when a buffer carries no metadata (the section is omitted entirely
//! when `metadata_size == 0`). Reading a v4 stream that *does* contain metadata
//! is also handled: the bytes are skipped (they cannot be reconstructed
//! without the Variant codec). Direct [`deserialize`] returns
//! [`Error::MetadataSkipped`] after loading voxel data so callers can surface a
//! warning. [`decompress_and_deserialize`] treats that same condition as
//! non-fatal, matching the C++ wrapper-style load path.
//!
//! Legacy version-2/3 migration paths depend on the same Godot Variant codec
//! and are therefore deferred as well.

use crate::io::serialization::{MemoryReader, MemoryWriter};
use crate::storage::voxel_buffer::{Compression, MAX_CHANNELS, MAX_SIZE};
use crate::storage::{ChannelDepth, VoxelBuffer};
use crate::streams::compressed_data;

/// Latest on-disk version, written by [`serialize`].
pub const BLOCK_FORMAT_VERSION: u8 = 4;

/// Trailing sanity-check word. `0x900df00d` ("good food"). Matches C++.
pub const BLOCK_TRAILING_MAGIC: u32 = 0x900df00d;
const BLOCK_TRAILING_MAGIC_SIZE: usize = 4;

/// Why (de)serialization failed. Mirrors the `false` returns / `ERR_FAIL_COND`
/// paths in the C++.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Tried to serialize a buffer with zero volume (C++ refuses this).
    EmptyBuffer,
    /// A buffer dimension exceeds `MAX_SIZE` (won't fit a `u16`).
    SizeOverflow,
    /// Reader ran out of bytes mid-field.
    UnexpectedEof,
    /// Trailing `0x900df00d` mismatch — the stream is corrupt or truncated.
    BadTrailingMagic { expected: u32, found: u32 },
    /// Tag/version/compression/depth byte outside the valid range.
    InvalidFormat(String),
    /// Unsupported on-disk version (v2/v3 migration needs the Godot Variant
    /// codec, which is not yet ported).
    UnsupportedVersion(u8),
    /// The stream declared a metadata section, which this build cannot decode
    /// (the Variant/custom-metadata subsystem is not ported). The voxel data
    /// itself is still loaded.
    MetadataSkipped,
    /// Compression envelope failure (LZ4/ZSTD).
    Compress(compressed_data::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::EmptyBuffer => write!(f, "block_serializer: cannot serialize empty buffer"),
            Error::SizeOverflow => write!(f, "block_serializer: buffer dimension exceeds MAX_SIZE"),
            Error::UnexpectedEof => write!(f, "block_serializer: unexpected end of stream"),
            Error::BadTrailingMagic { expected, found } => write!(
                f,
                "block_serializer: bad trailing magic (expected {expected:#x}, found {found:#x})"
            ),
            Error::InvalidFormat(m) => write!(f, "block_serializer: invalid format ({m})"),
            Error::UnsupportedVersion(v) => {
                write!(f, "block_serializer: unsupported version {v} (legacy migration needs Godot Variant codec)")
            }
            Error::MetadataSkipped => write!(
                f,
                "block_serializer: metadata section present but skipped (Variant codec not ported)"
            ),
            Error::Compress(e) => write!(f, "block_serializer: compression error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<compressed_data::Error> for Error {
    fn from(e: compressed_data::Error) -> Self {
        Error::Compress(e)
    }
}

/// Pack a channel's compression (low nibble) and depth (high nibble) into one
/// byte — the on-disk `fmt` field. Matches the C++ layout.
#[inline]
fn pack_format(compression: Compression, depth: ChannelDepth) -> u8 {
    (compression as u8) | ((depth as u8) << 4)
}

/// Unpack the `fmt` byte into (compression, depth). Returns `Err(bad_nibble)`
/// if either nibble is out of range — the nibble value is returned so the
/// caller can include it in an error message.
fn unpack_format(fmt: u8) -> Result<(Compression, ChannelDepth), u8> {
    let compression = match fmt & 0x0f {
        0 => Compression::None,
        1 => Compression::Uniform,
        other => return Err(other),
    };
    let depth = match (fmt >> 4) & 0x0f {
        0 => ChannelDepth::Bit8,
        1 => ChannelDepth::Bit16,
        2 => ChannelDepth::Bit32,
        3 => ChannelDepth::Bit64,
        other => return Err(other),
    };
    Ok((compression, depth))
}

// ---------------------------------------------------------------------------
// Serialize
// ---------------------------------------------------------------------------

/// Serialize `buffer` into `dst`, clearing it first. Returns the number of
/// bytes written. Ported from `BlockSerializer::serialize` (version 4, no
/// metadata section).
pub fn serialize(buffer: &VoxelBuffer, dst: &mut Vec<u8>) -> Result<usize, Error> {
    dst.clear();

    let size = buffer.size();
    if size.volume_u64() == 0 {
        return Err(Error::EmptyBuffer);
    }
    if size.x as u32 > MAX_SIZE || size.y as u32 > MAX_SIZE || size.z as u32 > MAX_SIZE {
        return Err(Error::SizeOverflow);
    }

    {
        let mut w = MemoryWriter::little(dst);
        w.store_8(BLOCK_FORMAT_VERSION);
        w.store_16(size.x as u16);
        w.store_16(size.y as u16);
        w.store_16(size.z as u16);

        for ci in 0..MAX_CHANNELS {
            let compression = buffer.channel_compression(ci);
            let depth = buffer.channel_depth(ci);
            w.store_8(pack_format(compression, depth));

            match compression {
                Compression::None => {
                    let bytes = buffer.channel_bytes(ci);
                    w.store_buffer(bytes);
                }
                Compression::Uniform => {
                    // C++ reads the voxel at (0,0,0); for a uniform channel
                    // every voxel equals the default value.
                    let v = buffer.channel_default(ci);
                    store_raw_by_depth(&mut w, v, depth);
                }
            }
        }
        // No metadata section is written — byte-compatible with C++ when the
        // buffer has no metadata (the section is omitted entirely).
        w.store_32(BLOCK_TRAILING_MAGIC);
    }

    Ok(dst.len())
}

/// Write a single raw voxel value using the width implied by `depth`.
/// Mirrors the `switch (depth)` blocks in `serialize`.
fn store_raw_by_depth(w: &mut MemoryWriter<'_, Vec<u8>>, v: u64, depth: ChannelDepth) {
    match depth {
        ChannelDepth::Bit8 => w.store_8(v as u8),
        ChannelDepth::Bit16 => w.store_16(v as u16),
        ChannelDepth::Bit32 => w.store_32(v as u32),
        ChannelDepth::Bit64 => w.store_64(v),
    }
}

/// Read a single raw voxel value with the width implied by `depth`.
fn read_raw_by_depth(r: &mut MemoryReader<'_>, depth: ChannelDepth) -> Option<u64> {
    match depth {
        ChannelDepth::Bit8 => r.try_get_8().map(|v| v as u64),
        ChannelDepth::Bit16 => r.try_get_16().map(|v| v as u64),
        ChannelDepth::Bit32 => r.try_get_32().map(|v| v as u64),
        ChannelDepth::Bit64 => r.try_get_64(),
    }
}

// ---------------------------------------------------------------------------
// Deserialize
// ---------------------------------------------------------------------------

/// Deserialize `src` into `buffer`, re-creating it. If a version-4 metadata
/// section is present, voxel data is loaded and [`Error::MetadataSkipped`] is
/// returned so the caller can decide whether to treat it as a warning. Legacy
/// v2/v3 migration is deferred — see the module docs.
pub fn deserialize(src: &[u8], buffer: &mut VoxelBuffer) -> Result<(), Error> {
    // Quick corruption check: the last 4 bytes must be the trailing magic.
    if src.len() < BLOCK_TRAILING_MAGIC_SIZE {
        return Err(Error::UnexpectedEof);
    }
    let tail_start = src.len() - BLOCK_TRAILING_MAGIC_SIZE;
    let magic = u32::from_le_bytes([
        src[tail_start],
        src[tail_start + 1],
        src[tail_start + 2],
        src[tail_start + 3],
    ]);
    if magic != BLOCK_TRAILING_MAGIC {
        return Err(Error::BadTrailingMagic {
            expected: BLOCK_TRAILING_MAGIC,
            found: magic,
        });
    }

    let mut r = MemoryReader::little(src);
    let version = r.try_get_8().ok_or(Error::UnexpectedEof)?;
    if version != BLOCK_FORMAT_VERSION {
        // v2/v3 migration needs the Godot Variant metadata codec — deferred.
        return Err(Error::UnsupportedVersion(version));
    }

    let size_x = r.try_get_16().ok_or(Error::UnexpectedEof)? as i32;
    let size_y = r.try_get_16().ok_or(Error::UnexpectedEof)? as i32;
    let size_z = r.try_get_16().ok_or(Error::UnexpectedEof)? as i32;
    buffer.create(crate::math::Vector3i::new(size_x, size_y, size_z));

    for ci in 0..MAX_CHANNELS {
        let fmt = r.try_get_8().ok_or(Error::UnexpectedEof)?;
        let (compression, depth) = unpack_format(fmt).map_err(|bad| {
            Error::InvalidFormat(format!(
                "channel {ci}: bad fmt byte {fmt:#x} (bad nibble {bad})"
            ))
        })?;
        buffer.set_channel_depth(ci, depth);

        match compression {
            Compression::None => {
                // Decompress (uniform → allocated) so we can write voxel bytes.
                buffer.decompress_channel(ci);
                let dst_bytes = buffer.channel_bytes_mut(ci);
                let expected = dst_bytes.len();
                let src_slice = r.try_take(expected).ok_or(Error::UnexpectedEof)?;
                dst_bytes.copy_from_slice(src_slice);
            }
            Compression::Uniform => {
                let v = read_raw_by_depth(&mut r, depth).ok_or(Error::UnexpectedEof)?;
                buffer.clear_channel(ci, v);
            }
        }
    }

    // Anything between the channels and the trailing magic must be a metadata
    // section encoded as `[u32 size][size bytes]`. This port can't reconstruct
    // it (Variant codec not ported), but it must still validate the envelope so
    // trailing junk doesn't pass as a non-fatal metadata warning.
    let remaining_before_magic =
        (src.len() - BLOCK_TRAILING_MAGIC_SIZE).saturating_sub(r.position());
    if remaining_before_magic > 0 {
        if remaining_before_magic < 4 {
            return Err(Error::UnexpectedEof);
        }
        let metadata_pos = r.position();
        let metadata_size = u32::from_le_bytes([
            src[metadata_pos],
            src[metadata_pos + 1],
            src[metadata_pos + 2],
            src[metadata_pos + 3],
        ]) as usize;
        let expected_metadata_section_len = 4usize
            .checked_add(metadata_size)
            .ok_or_else(|| Error::InvalidFormat("metadata section size overflow".to_string()))?;
        if expected_metadata_section_len != remaining_before_magic {
            return Err(Error::InvalidFormat(format!(
                "metadata section length mismatch (declared {metadata_size}, remaining {})",
                remaining_before_magic - 4
            )));
        }
        if metadata_size > 0 {
            return Err(Error::MetadataSkipped);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Compressed wrappers
// ---------------------------------------------------------------------------

/// Serialize `buffer`, then compress the result. Ported from
/// `BlockSerializer::serialize_and_compress`.
pub fn serialize_and_compress(
    buffer: &VoxelBuffer,
    dst: &mut Vec<u8>,
    compression_mode: compressed_data::Compression,
) -> Result<usize, Error> {
    let mut raw = Vec::new();
    serialize(buffer, &mut raw)?;
    compressed_data::compress(&raw, dst, compression_mode)?;
    Ok(dst.len())
}

/// Decompress `src`, then deserialize. Ported from
/// `BlockSerializer::decompress_and_deserialize`.
pub fn decompress_and_deserialize(src: &[u8], buffer: &mut VoxelBuffer) -> Result<(), Error> {
    let mut raw = Vec::new();
    compressed_data::decompress(src, &mut raw)?;
    // If the inner block carried metadata we couldn't decode, surface it as a
    // non-fatal warning: the voxel data is still loaded correctly.
    match deserialize(&raw, buffer) {
        Ok(()) => Ok(()),
        Err(Error::MetadataSkipped) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vector3i;
    use crate::storage::voxel_buffer::Allocator;

    /// Build a small buffer with a few distinct voxel values in channel 0
    /// (so it stays non-uniform) and a uniform value in channel 1.
    fn sample_buffer() -> VoxelBuffer {
        let mut b = VoxelBuffer::with_size(Vector3i::new(4, 2, 3));
        // Non-uniform channel 0.
        for z in 0..3 {
            for y in 0..2 {
                for x in 0..4 {
                    b.set_voxel(((x + y * 4 + z * 8) as u64) & 0xff, x, y, z, 0);
                }
            }
        }
        // Uniform channel 1 (default Compression::Uniform).
        b.clear_channel(1, 42);
        b
    }

    fn append_metadata_section(bytes: &mut Vec<u8>, metadata: &[u8]) {
        let magic = bytes.split_off(bytes.len() - BLOCK_TRAILING_MAGIC_SIZE);
        bytes.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
        bytes.extend_from_slice(metadata);
        bytes.extend_from_slice(&magic);
    }

    #[test]
    fn serialize_round_trips_structure() {
        let src = sample_buffer();
        let mut bytes = Vec::new();
        let n = serialize(&src, &mut bytes).unwrap();
        assert!(n > 0);

        let mut dst = VoxelBuffer::new(Allocator::Default);
        deserialize(&bytes, &mut dst).unwrap();

        assert_eq!(dst.size(), Vector3i::new(4, 2, 3));
        for z in 0..3 {
            for y in 0..2 {
                for x in 0..4 {
                    assert_eq!(
                        dst.get_voxel(x, y, z, 0),
                        src.get_voxel(x, y, z, 0),
                        "ch0 at ({x},{y},{z})"
                    );
                }
            }
        }
        // Channel 1 uniform value round-trips.
        assert_eq!(dst.get_voxel(0, 0, 0, 1), 42);
    }

    #[test]
    fn serialize_writes_version_and_trailing_magic() {
        let mut bytes = Vec::new();
        serialize(&sample_buffer(), &mut bytes).unwrap();
        assert_eq!(bytes[0], BLOCK_FORMAT_VERSION);
        let n = bytes.len();
        let magic = u32::from_le_bytes([bytes[n - 4], bytes[n - 3], bytes[n - 2], bytes[n - 1]]);
        assert_eq!(magic, BLOCK_TRAILING_MAGIC);
    }

    #[test]
    fn serialize_rejects_empty_buffer() {
        let empty = VoxelBuffer::new(Allocator::Default); // size (0,0,0)
        let mut bytes = Vec::new();
        assert_eq!(serialize(&empty, &mut bytes), Err(Error::EmptyBuffer));
    }

    #[test]
    fn deserialize_rejects_bad_trailing_magic() {
        let mut bytes = Vec::new();
        serialize(&sample_buffer(), &mut bytes).unwrap();
        // Corrupt the trailing magic.
        let n = bytes.len();
        bytes[n - 1] ^= 0xff;
        let mut dst = VoxelBuffer::new(Allocator::Default);
        match deserialize(&bytes, &mut dst) {
            Err(Error::BadTrailingMagic { .. }) => {}
            other => panic!("expected BadTrailingMagic, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_rejects_unsupported_version() {
        // Build a stream that begins with version 2.
        let mut bytes = Vec::new();
        bytes.push(2u8);
        // Pad to at least the trailing-magic length so the early magic check
        // passes; place a valid-looking magic at the end.
        bytes.extend_from_slice(&[0u8; 3]);
        bytes.extend_from_slice(&BLOCK_TRAILING_MAGIC.to_le_bytes());
        let mut dst = VoxelBuffer::new(Allocator::Default);
        assert_eq!(
            deserialize(&bytes, &mut dst),
            Err(Error::UnsupportedVersion(2))
        );
    }

    #[test]
    fn pack_unpack_format_round_trips() {
        for compression in [Compression::None, Compression::Uniform] {
            for depth in [
                ChannelDepth::Bit8,
                ChannelDepth::Bit16,
                ChannelDepth::Bit32,
                ChannelDepth::Bit64,
            ] {
                let fmt = pack_format(compression, depth);
                let (c, d) = unpack_format(fmt).unwrap();
                assert_eq!(c, compression);
                assert_eq!(d, depth);
            }
        }
    }

    #[test]
    fn unpack_format_rejects_out_of_range_nibbles() {
        assert!(unpack_format(0x02).is_err()); // bad compression
        assert!(unpack_format(0x40).is_err()); // bad depth
        assert!(unpack_format(0xff).is_err());
    }

    #[test]
    fn uniform_channel_serializes_as_single_value() {
        let mut src = VoxelBuffer::with_size(Vector3i::new(8, 8, 8));
        src.clear_channel(0, 123); // uniform
        let mut bytes = Vec::new();
        serialize(&src, &mut bytes).unwrap();

        let mut dst = VoxelBuffer::new(Allocator::Default);
        deserialize(&bytes, &mut dst).unwrap();
        // Every voxel in channel 0 is 123.
        for z in 0..8 {
            for y in 0..8 {
                for x in 0..8 {
                    assert_eq!(dst.get_voxel(x, y, z, 0), 123);
                }
            }
        }
        assert_eq!(dst.channel_compression(0), Compression::Uniform);
    }

    #[test]
    fn depth_16bit_channel_round_trips() {
        let mut src = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        src.set_channel_depth(0, ChannelDepth::Bit16);
        src.decompress_channel(0);
        // Write distinct 16-bit values.
        src.set_voxel(0x1234, 0, 0, 0, 0);
        src.set_voxel(0xabcd, 1, 0, 0, 0);
        let mut bytes = Vec::new();
        serialize(&src, &mut bytes).unwrap();

        let mut dst = VoxelBuffer::new(Allocator::Default);
        deserialize(&bytes, &mut dst).unwrap();
        assert_eq!(dst.channel_depth(0), ChannelDepth::Bit16);
        assert_eq!(dst.get_voxel(0, 0, 0, 0), 0x1234);
        assert_eq!(dst.get_voxel(1, 0, 0, 0), 0xabcd);
    }

    #[test]
    fn serialize_and_compress_round_trips_with_lz4() {
        let src = sample_buffer();
        let mut compressed = Vec::new();
        serialize_and_compress(&src, &mut compressed, compressed_data::Compression::Lz4).unwrap();

        let mut dst = VoxelBuffer::new(Allocator::Default);
        decompress_and_deserialize(&compressed, &mut dst).unwrap();
        assert_eq!(dst.size(), Vector3i::new(4, 2, 3));
        for z in 0..3 {
            for y in 0..2 {
                for x in 0..4 {
                    assert_eq!(dst.get_voxel(x, y, z, 0), src.get_voxel(x, y, z, 0));
                }
            }
        }
    }

    #[test]
    fn compressed_round_trip_with_none_compression() {
        let src = sample_buffer();
        let mut wrapped = Vec::new();
        serialize_and_compress(&src, &mut wrapped, compressed_data::Compression::None).unwrap();

        let mut dst = VoxelBuffer::new(Allocator::Default);
        decompress_and_deserialize(&wrapped, &mut dst).unwrap();
        assert_eq!(dst.size(), src.size());
    }

    #[test]
    fn direct_deserialize_reports_metadata_after_loading_voxels() {
        let src = sample_buffer();
        let mut bytes = Vec::new();
        serialize(&src, &mut bytes).unwrap();
        append_metadata_section(&mut bytes, b"metadata");

        let mut dst = VoxelBuffer::new(Allocator::Default);
        assert_eq!(deserialize(&bytes, &mut dst), Err(Error::MetadataSkipped));
        assert_eq!(dst.size(), src.size());
        assert_eq!(dst.get_voxel(3, 1, 2, 0), src.get_voxel(3, 1, 2, 0));
    }

    #[test]
    fn deserialize_rejects_trailing_junk_before_magic() {
        let mut bytes = Vec::new();
        serialize(&sample_buffer(), &mut bytes).unwrap();
        let magic = bytes.split_off(bytes.len() - BLOCK_TRAILING_MAGIC_SIZE);
        bytes.push(0xff);
        bytes.extend_from_slice(&magic);

        let mut dst = VoxelBuffer::new(Allocator::Default);
        assert_eq!(deserialize(&bytes, &mut dst), Err(Error::UnexpectedEof));
    }

    #[test]
    fn deserialize_rejects_metadata_size_mismatch() {
        let mut bytes = Vec::new();
        serialize(&sample_buffer(), &mut bytes).unwrap();
        let magic = bytes.split_off(bytes.len() - BLOCK_TRAILING_MAGIC_SIZE);
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(b"xy");
        bytes.extend_from_slice(&magic);

        let mut dst = VoxelBuffer::new(Allocator::Default);
        match deserialize(&bytes, &mut dst) {
            Err(Error::InvalidFormat(message)) => {
                assert!(message.contains("metadata section length mismatch"));
            }
            other => panic!("expected metadata size mismatch, got {other:?}"),
        }
    }

    #[test]
    fn compressed_wrapper_rejects_malformed_metadata_envelope() {
        let mut raw = Vec::new();
        serialize(&sample_buffer(), &mut raw).unwrap();
        let magic = raw.split_off(raw.len() - BLOCK_TRAILING_MAGIC_SIZE);
        raw.push(0xff);
        raw.extend_from_slice(&magic);

        let mut wrapped = Vec::new();
        compressed_data::compress(&raw, &mut wrapped, compressed_data::Compression::None).unwrap();

        let mut dst = VoxelBuffer::new(Allocator::Default);
        assert_eq!(
            decompress_and_deserialize(&wrapped, &mut dst),
            Err(Error::UnexpectedEof)
        );
    }
}
