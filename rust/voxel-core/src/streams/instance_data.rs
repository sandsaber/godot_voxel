//! `streams::instance_data` — lossy-compressed per-block instance transforms.
//!
//! Ported from `streams/instance_data.{h,cpp}`. Stores instanced-grass /
//! instanced-detail data per voxel block in a compact on-disk format
//! (`FORMAT_SIMPLE_11B_V1`: position → 3×u16, scale → u8, rotation → 4×u8
//! quaternion). The format is internal to godot_voxel (not a public asset
//! format like `.vox`), so this module owns its versioning.
//!
//! The C++ reader returns `bool` from `deserialize_*`; the Rust port returns
//! `Result<_, DeserializeError>` for idiomatic error propagation, with
//! [`DeserializeError`] distinguishing truncation, version mismatch and a
//! wrong trailing magic.

use crate::io::serialization::{Endianness, MemoryReader, MemoryWriter};
use crate::math::funcs;
use crate::math::{Basis3f, Quaternionf, Transform3f, Vector3f};

/// Trailing sanity-check written after every block. `0x900df00d` ("good food").
const TRAILING_MAGIC: u32 = 0x900df00d;

/// On-disk format versions. Version 0 was big-endian (deprecated); version 1
/// switched to little-endian. Ported from the anonymous `FormatVersion` enum.
const FORMAT_VERSION_0: u8 = 0;
const FORMAT_VERSION_1: u8 = 1;

/// `InstanceBlockData::POSITION_RESOLUTION`.
pub const POSITION_RESOLUTION: i32 = 65536;
/// `InstanceBlockData::POSITION_RANGE_MINIMUM` — clamped so the u16 position
/// quantization never divides by zero.
pub const POSITION_RANGE_MINIMUM: f32 = 0.01;
/// `InstanceBlockData::SIMPLE_11B_V1_SCALE_RESOLUTION`.
pub const SIMPLE_11B_V1_SCALE_RESOLUTION: i32 = 256;
/// `InstanceBlockData::SIMPLE_11B_V1_SCALE_RANGE_MINIMUM`.
pub const SIMPLE_11B_V1_SCALE_RANGE_MINIMUM: f32 = 0.01;

/// `1.0 / 0x7f` — matches `constants::INV_0x7f` used by `u8_to_norm`.
const INV_0X7F: f32 = 1.0 / 127.0;

/// Why deserialization failed. Mirrors the `false`-return paths in the C++
/// `deserialize_instance_block_data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeserializeError {
    /// Reader ran out of bytes mid-field.
    UnexpectedEof,
    /// First byte was neither version 0 nor version 1.
    UnsupportedVersion(u8),
    /// `instance_format` byte was not `FORMAT_SIMPLE_11B_V1`.
    UnsupportedInstanceFormat(u8),
    /// A range invariant failed (e.g. `scale_max < scale_min`).
    InvalidRange(&'static str),
    /// Trailing `0x900df00d` mismatch — the stream was truncated or corrupt.
    BadTrailingMagic { expected: u32, found: u32 },
}

impl std::fmt::Display for DeserializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeserializeError::UnexpectedEof => write!(f, "instance_data: unexpected end of stream"),
            DeserializeError::UnsupportedVersion(v) => {
                write!(f, "instance_data: unsupported version {v}")
            }
            DeserializeError::UnsupportedInstanceFormat(v) => {
                write!(f, "instance_data: unsupported instance format {v}")
            }
            DeserializeError::InvalidRange(m) => write!(f, "instance_data: invalid range ({m})"),
            DeserializeError::BadTrailingMagic { expected, found } => write!(
                f,
                "instance_data: bad trailing magic (expected {expected:#x}, found {found:#x})"
            ),
        }
    }
}

impl std::error::Error for DeserializeError {}

/// Which on-disk layout a layer uses. Only `Simple11bV1` is defined today.
/// Ported from `VoxelInstanceFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum VoxelInstanceFormat {
    #[default]
    /// Position → 3×u16, scale → u8, rotation → 4×u8 quaternion.
    Simple11bV1 = 0,
}

/// A single instance: just a transform relative to the block origin.
/// Ported from `InstanceBlockData::InstanceData`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InstanceData {
    pub transform: Transform3f,
}

/// One layer of instances sharing a scale range and an id.
/// Ported from `InstanceBlockData::LayerData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerData {
    pub id: u16,
    pub scale_min: f32,
    pub scale_max: f32,
    pub instances: Vec<InstanceData>,
}

/// The whole block: a position-quantization range plus a list of layers.
/// Ported from `InstanceBlockData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstanceBlockData {
    pub position_range: f32,
    pub layers: Vec<LayerData>,
}

/// A quaternion quantized to four bytes (-1..1 → 0..255).
/// Ported from the anonymous `CompressedQuaternion4b` struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompressedQuaternion4b {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub w: u8,
}

impl CompressedQuaternion4b {
    /// Quantize a unit quaternion. Matches `from_quaternion`.
    pub fn from_quaternion(q: Quaternionf) -> Self {
        Self {
            x: norm_to_u8(q.x),
            y: norm_to_u8(q.y),
            z: norm_to_u8(q.z),
            w: norm_to_u8(q.w),
        }
    }

    /// Dequantize back to a unit quaternion. Matches `to_quaternion`.
    pub fn to_quaternion(self) -> Quaternionf {
        let q = Quaternionf::new(
            u8_to_norm(self.x),
            u8_to_norm(self.y),
            u8_to_norm(self.z),
            u8_to_norm(self.w),
        );
        crate::math::quaternion::math::normalized(q)
    }
}

/// Map a normalized float (`-1..1`) to `0..255`. Matches `norm_to_u8`.
#[inline]
fn norm_to_u8(x: f32) -> u8 {
    funcs::clamp((128.0_f32 * x + 128.0) as i32, 0, 0xff) as u8
}

/// Inverse of [`norm_to_u8`]. Matches `u8_to_norm`.
#[inline]
fn u8_to_norm(v: u8) -> f32 {
    (v as f32 - 0x7f as f32) * INV_0X7F
}

/// Serialize an [`InstanceBlockData`] into `dst`. Returns `false` (matching the
/// C++ `bool` return) if a precondition fails; on success appends the payload
/// and returns `true`.
///
/// Ported from `serialize_instance_block_data`. Always writes version 1
/// (little-endian).
pub fn serialize(src: &InstanceBlockData, dst: &mut Vec<u8>) -> bool {
    let instance_format = VoxelInstanceFormat::Simple11bV1 as u8;

    let mut w = MemoryWriter::little(dst);

    if src.position_range < 0.0 {
        return false;
    }
    let position_range = funcs::max(src.position_range, POSITION_RANGE_MINIMUM);

    w.store_8(FORMAT_VERSION_1);
    // `src.layers.len()` fits a u8 in practice (godot_voxel caps layer count
    // well below 256); the C++ code stores the same byte.
    w.store_8(src.layers.len() as u8);
    w.store_float(position_range);

    let pos_norm_scale = 1.0 / position_range;

    for layer in &src.layers {
        if layer.scale_max < layer.scale_min {
            return false;
        }
        // Guarantee a non-zero scale range so the u8 quantization doesn't
        // collapse to a single value.
        let (scale_min, scale_max) =
            if layer.scale_max - layer.scale_min < SIMPLE_11B_V1_SCALE_RANGE_MINIMUM {
                let lo = layer.scale_min;
                (lo, lo + SIMPLE_11B_V1_SCALE_RANGE_MINIMUM)
            } else {
                (layer.scale_min, layer.scale_max)
            };

        w.store_16(layer.id);
        w.store_16(layer.instances.len() as u16);
        w.store_float(scale_min);
        w.store_float(scale_max);
        w.store_8(instance_format);

        let scale_norm_scale = 1.0 / (scale_max - scale_min);

        for instance in &layer.instances {
            let o = instance.transform.origin;
            w.store_16((pos_norm_scale * o.x * 0xffff as f32) as u16);
            w.store_16((pos_norm_scale * o.y * 0xffff as f32) as u16);
            w.store_16((pos_norm_scale * o.z * 0xffff as f32) as u16);

            let scale = instance.transform.basis.get_scale_abs().y;
            w.store_8((scale_norm_scale * (scale - scale_min) * 0xff as f32) as u8);

            let q = instance.transform.basis.get_rotation_quaternion();
            let cq = CompressedQuaternion4b::from_quaternion(q);
            w.store_8(cq.x);
            w.store_8(cq.y);
            w.store_8(cq.z);
            w.store_8(cq.w);
        }
    }

    w.store_32(TRAILING_MAGIC);
    true
}

/// Deserialize an [`InstanceBlockData`] from `src`. Ported from
/// `deserialize_instance_block_data`. Version 0 (big-endian, legacy) is
/// accepted for read-back compatibility, exactly as in C++.
pub fn deserialize(dst: &mut InstanceBlockData, src: &[u8]) -> Result<(), DeserializeError> {
    let expected_instance_format = VoxelInstanceFormat::Simple11bV1 as u8;
    let mut r = MemoryReader::little(src);

    let version = r.try_get_8().ok_or(DeserializeError::UnexpectedEof)?;
    // Legacy v0 was big-endian; the C++ reader flips `r.endianness` in place.
    if version == FORMAT_VERSION_0 {
        r.set_endianness(Endianness::BigEndian);
    } else if version != FORMAT_VERSION_1 {
        return Err(DeserializeError::UnsupportedVersion(version));
    }

    let layers_count = r.try_get_8().ok_or(DeserializeError::UnexpectedEof)? as usize;
    dst.layers.clear();
    dst.layers.resize_with(layers_count, LayerData::default);

    dst.position_range = r.try_get_float().ok_or(DeserializeError::UnexpectedEof)?;

    for layer in &mut dst.layers {
        layer.id = r.try_get_16().ok_or(DeserializeError::UnexpectedEof)?;

        let instance_count = r.try_get_16().ok_or(DeserializeError::UnexpectedEof)? as usize;
        layer.instances.clear();
        layer
            .instances
            .resize_with(instance_count, InstanceData::default);

        layer.scale_min = r.try_get_float().ok_or(DeserializeError::UnexpectedEof)?;
        layer.scale_max = r.try_get_float().ok_or(DeserializeError::UnexpectedEof)?;
        if layer.scale_max < layer.scale_min {
            return Err(DeserializeError::InvalidRange("scale_max < scale_min"));
        }
        let scale_range = layer.scale_max - layer.scale_min;

        let instance_format = r.try_get_8().ok_or(DeserializeError::UnexpectedEof)?;
        if instance_format != expected_instance_format {
            return Err(DeserializeError::UnsupportedInstanceFormat(instance_format));
        }

        for instance in &mut layer.instances {
            let x = (r.try_get_16().ok_or(DeserializeError::UnexpectedEof)? as f32 / 0xffff as f32)
                * dst.position_range;
            let y = (r.try_get_16().ok_or(DeserializeError::UnexpectedEof)? as f32 / 0xffff as f32)
                * dst.position_range;
            let z = (r.try_get_16().ok_or(DeserializeError::UnexpectedEof)? as f32 / 0xffff as f32)
                * dst.position_range;

            let s = (r.try_get_8().ok_or(DeserializeError::UnexpectedEof)? as f32 / 0xff as f32)
                * scale_range
                + layer.scale_min;

            let cq = CompressedQuaternion4b {
                x: r.try_get_8().ok_or(DeserializeError::UnexpectedEof)?,
                y: r.try_get_8().ok_or(DeserializeError::UnexpectedEof)?,
                z: r.try_get_8().ok_or(DeserializeError::UnexpectedEof)?,
                w: r.try_get_8().ok_or(DeserializeError::UnexpectedEof)?,
            };
            let q = cq.to_quaternion();

            instance.transform = Transform3f::new(
                Basis3f::from_quaternion(q).scaled(s),
                Vector3f::new(x, y, z),
            );
        }
    }

    let control_end = r.try_get_32().ok_or(DeserializeError::UnexpectedEof)?;
    if control_end != TRAILING_MAGIC {
        return Err(DeserializeError::BadTrailingMagic {
            expected: TRAILING_MAGIC,
            found: control_end,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small block with one layer + a couple of rotated, scaled
    /// instances, used by several round-trip checks.
    fn sample_block() -> InstanceBlockData {
        let mut layer = LayerData {
            id: 3,
            scale_min: 0.5,
            scale_max: 2.0,
            instances: Vec::new(),
        };
        // Two instances: identity-ish and a 90° rotation about Y, scaled 1.5×.
        layer.instances.push(InstanceData {
            transform: Transform3f::new(
                Basis3f::from_quaternion(Quaternionf::new(0.0, 0.0, 0.0, 1.0)).scaled(1.0),
                Vector3f::new(0.25, 0.5, 0.75),
            ),
        });
        let half_root_two = std::f32::consts::FRAC_1_SQRT_2;
        layer.instances.push(InstanceData {
            transform: Transform3f::new(
                Basis3f::from_quaternion(Quaternionf::new(0.0, half_root_two, 0.0, half_root_two))
                    .scaled(1.5),
                Vector3f::new(1.0, 2.0, 3.0),
            ),
        });
        InstanceBlockData {
            position_range: 4.0,
            layers: vec![layer],
        }
    }

    #[test]
    fn serialize_deserialize_roundtrips_structure() {
        let src = sample_block();
        let mut bytes = Vec::new();
        assert!(serialize(&src, &mut bytes));

        let mut dst = InstanceBlockData::default();
        deserialize(&mut dst, &bytes).expect("round-trip should succeed");

        assert_eq!(dst.position_range, 4.0);
        assert_eq!(dst.layers.len(), 1);
        assert_eq!(dst.layers[0].id, 3);
        assert_eq!(dst.layers[0].scale_min, 0.5);
        assert_eq!(dst.layers[0].scale_max, 2.0);
        assert_eq!(dst.layers[0].instances.len(), 2);
    }

    #[test]
    fn roundtrip_preserves_origin_within_quantization_tolerance() {
        // Position is quantized to u16 over `position_range`; tolerance is one
        // quantum = position_range / 65535.
        let src = sample_block();
        let mut bytes = Vec::new();
        assert!(serialize(&src, &mut bytes));
        let mut dst = InstanceBlockData::default();
        deserialize(&mut dst, &bytes).unwrap();

        let tol = src.position_range / (POSITION_RESOLUTION as f32 - 1.0) + 1e-4;
        for (a, b) in src.layers[0]
            .instances
            .iter()
            .zip(dst.layers[0].instances.iter())
        {
            assert!(
                (a.transform.origin.x - b.transform.origin.x).abs() < tol,
                "x diverged"
            );
            assert!(
                (a.transform.origin.y - b.transform.origin.y).abs() < tol,
                "y diverged"
            );
            assert!(
                (a.transform.origin.z - b.transform.origin.z).abs() < tol,
                "z diverged"
            );
        }
    }

    #[test]
    fn roundtrip_preserves_scale_within_quantization_tolerance() {
        let src = sample_block();
        let mut bytes = Vec::new();
        assert!(serialize(&src, &mut bytes));
        let mut dst = InstanceBlockData::default();
        deserialize(&mut dst, &bytes).unwrap();

        let scale_range = src.layers[0].scale_max - src.layers[0].scale_min;
        let tol = scale_range / (SIMPLE_11B_V1_SCALE_RESOLUTION - 1) as f32 + 1e-4;
        for (a, b) in src.layers[0]
            .instances
            .iter()
            .zip(dst.layers[0].instances.iter())
        {
            let sa = a.transform.basis.get_scale_abs().y;
            let sb = b.transform.basis.get_scale_abs().y;
            assert!((sa - sb).abs() < tol, "scale diverged: {sa} vs {sb}");
        }
    }

    #[test]
    fn serialize_writes_trailing_magic() {
        let mut bytes = Vec::new();
        assert!(serialize(&sample_block(), &mut bytes));
        // Last 4 bytes (little-endian) must be the magic.
        let n = bytes.len();
        let tail = u32::from_le_bytes([bytes[n - 4], bytes[n - 3], bytes[n - 2], bytes[n - 1]]);
        assert_eq!(tail, TRAILING_MAGIC);
    }

    #[test]
    fn serialize_rejects_negative_position_range() {
        let mut bad = sample_block();
        bad.position_range = -1.0;
        let mut bytes = Vec::new();
        assert!(!serialize(&bad, &mut bytes));
    }

    #[test]
    fn serialize_rejects_inverted_scale_range() {
        let mut bad = sample_block();
        bad.layers[0].scale_min = 5.0;
        bad.layers[0].scale_max = 1.0;
        let mut bytes = Vec::new();
        // The min-range clamp in serialize does NOT rescue an inverted range;
        // the C++ `scale_max >= scale_min` check fails first.
        assert!(!serialize(&bad, &mut bytes));
    }

    #[test]
    fn serialize_clamps_tiny_scale_range_to_minimum() {
        // A vanishingly small range is widened to SIMPLE_11B_V1_SCALE_RANGE_MINIMUM.
        let mut narrow = sample_block();
        narrow.layers[0].scale_min = 1.0;
        narrow.layers[0].scale_max = 1.00001;
        let mut bytes = Vec::new();
        assert!(
            serialize(&narrow, &mut bytes),
            "clamp should let serialize succeed"
        );

        let mut dst = InstanceBlockData::default();
        deserialize(&mut dst, &bytes).unwrap();
        // The reader sees the widened range.
        let range = dst.layers[0].scale_max - dst.layers[0].scale_min;
        assert!(
            range >= SIMPLE_11B_V1_SCALE_RANGE_MINIMUM - 1e-6,
            "scale range was not clamped up: {range}"
        );
    }

    #[test]
    fn deserialize_rejects_truncated_stream() {
        let mut bytes = Vec::new();
        assert!(serialize(&sample_block(), &mut bytes));
        // Drop everything after the first byte.
        bytes.truncate(1);
        let mut dst = InstanceBlockData::default();
        assert_eq!(
            deserialize(&mut dst, &bytes),
            Err(DeserializeError::UnexpectedEof)
        );
    }

    #[test]
    fn deserialize_rejects_unsupported_version() {
        let mut bytes = vec![99u8]; // version byte only
                                    // Pad so the reader doesn't trip EOF before reaching the version check
                                    // (version is read first, so no padding is actually needed).
        bytes.extend_from_slice(&[0u8; 8]);
        let mut dst = InstanceBlockData::default();
        assert_eq!(
            deserialize(&mut dst, &bytes),
            Err(DeserializeError::UnsupportedVersion(99))
        );
    }

    #[test]
    fn deserialize_rejects_bad_trailing_magic() {
        let mut bytes = Vec::new();
        assert!(serialize(&sample_block(), &mut bytes));
        // Corrupt the trailing magic.
        let n = bytes.len();
        bytes[n - 1] ^= 0xff;
        let mut dst = InstanceBlockData::default();
        match deserialize(&mut dst, &bytes) {
            Err(DeserializeError::BadTrailingMagic { .. }) => {}
            other => panic!("expected BadTrailingMagic, got {other:?}"),
        }
    }

    #[test]
    fn norm_to_u8_round_trips_within_one_quant() {
        // `norm_to_u8` truncates rather than rounds, so the worst-case error is
        // just under two quanta (1/127). Allow a little headroom on top.
        for x in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            let v = norm_to_u8(x);
            let y = u8_to_norm(v);
            assert!(
                (x - y).abs() <= 2.0 * INV_0X7F + 1e-4,
                "norm {x} -> {v} -> {y}"
            );
        }
    }

    #[test]
    fn norm_to_u8_clamps_out_of_range() {
        // `clamp` keeps the byte in 0..255 even for inputs outside -1..1.
        assert_eq!(norm_to_u8(2.0), 0xff);
        assert_eq!(norm_to_u8(-2.0), 0);
    }

    #[test]
    fn empty_block_round_trips() {
        let src = InstanceBlockData {
            position_range: 1.0,
            layers: Vec::new(),
        };
        let mut bytes = Vec::new();
        assert!(serialize(&src, &mut bytes));
        let mut dst = InstanceBlockData::default();
        deserialize(&mut dst, &bytes).unwrap();
        assert_eq!(dst.layers.len(), 0);
        assert_eq!(dst.position_range, 1.0);
    }
}
