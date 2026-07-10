# Wave 0B Bounded Decode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add explicit decode limits to compression, voxel block deserialization, region block loading, and VOX parsing so untrusted sizes return bounded errors before large allocation.

**Architecture:** Introduce `DecodeLimits` as a small copyable policy object in `streams`, then add `_with_limits` entry points while preserving existing public wrappers with defaults. Every path checks declared byte, voxel, model, node, and string sizes before allocation or buffer creation.

**Tech Stack:** Rust, `voxel-core`, `lz4_flex`, optional `zstd`, existing unit tests.

## Global Constraints

- Preserve existing `decompress`, `deserialize`, `decompress_and_deserialize`, `RegionFile::load_block`, and `vox::parse` APIs by making them default-limit wrappers.
- New bounded APIs must return typed `Err`, not panic, for oversized declared inputs.
- Use fallible reservation before resizing vectors from untrusted declared sizes.
- Do not implement VOX chunk-framing parity in Wave 0B; that remains later audit scope.
- Do not touch `rust/AUDIT.md`; it is user-owned working-tree state.

---

## File Structure

- Create: `rust/voxel-core/src/streams/decode_limits.rs`
  - Own `DecodeLimits` defaults and helper checks shared by stream formats and VOX.
- Modify: `rust/voxel-core/src/streams/mod.rs`
  - Export `decode_limits`.
- Modify: `rust/voxel-core/src/streams/compressed_data.rs`
  - Add limit-aware decompression and allocation failure errors.
- Modify: `rust/voxel-core/src/streams/block_serializer.rs`
  - Add limit-aware deserialize wrappers and check v4 block dimensions/channel payloads before `VoxelBuffer::create`.
- Modify: `rust/voxel-core/src/streams/region/region_file.rs`
  - Add `load_block_with_limits` and enforce payload size against sector allocation and decode limits.
- Modify: `rust/voxel-core/src/format/vox/parser.rs`
  - Add `parse_with_limits`, limit dense model allocation, XYZI voxel count, cumulative model count, scene node count, dictionary string bytes.
- Modify: `rust/voxel-core/src/format/vox/mod.rs`
  - Export `parse_with_limits` if parser exports are explicit there.
- Test: existing test modules in the modified files.

### Task 1: Add DecodeLimits Policy Object

**Files:**
- Create: `rust/voxel-core/src/streams/decode_limits.rs`
- Modify: `rust/voxel-core/src/streams/mod.rs`
- Test: `rust/voxel-core/src/streams/decode_limits.rs`

**Interfaces:**
- Produces: `pub struct DecodeLimits`, `DecodeLimits::trusted()`, `check_*` helpers, and `DecodeLimitError`.
- Consumers: `compressed_data`, `block_serializer`, `region_file`, `format::vox::parser`.

- [ ] **Step 1: Write the new limits module with tests**

Create `rust/voxel-core/src/streams/decode_limits.rs`:

```rust
//! Shared bounds for decoding untrusted voxel formats.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    pub max_bytes: usize,
    pub max_block_voxels: u64,
    pub max_region_blocks: usize,
    pub max_vox_models: usize,
    pub max_vox_total_voxels: u64,
    pub max_vox_nodes: usize,
    pub max_string_bytes: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_block_voxels: 256 * 256 * 256,
            max_region_blocks: 255 * 255 * 255,
            max_vox_models: 4096,
            max_vox_total_voxels: 64 * 1024 * 1024,
            max_vox_nodes: 65_536,
            max_string_bytes: 4096,
        }
    }
}

impl DecodeLimits {
    pub const fn trusted() -> Self {
        Self {
            max_bytes: usize::MAX,
            max_block_voxels: u64::MAX,
            max_region_blocks: usize::MAX,
            max_vox_models: usize::MAX,
            max_vox_total_voxels: u64::MAX,
            max_vox_nodes: usize::MAX,
            max_string_bytes: usize::MAX,
        }
    }

    pub fn check_bytes(self, label: &'static str, requested: usize) -> Result<(), DecodeLimitError> {
        if requested > self.max_bytes {
            return Err(DecodeLimitError::LimitExceeded {
                label,
                requested: requested as u128,
                limit: self.max_bytes as u128,
            });
        }
        Ok(())
    }

    pub fn check_block_voxels(self, requested: u64) -> Result<(), DecodeLimitError> {
        if requested > self.max_block_voxels {
            return Err(DecodeLimitError::LimitExceeded {
                label: "block voxels",
                requested: requested as u128,
                limit: self.max_block_voxels as u128,
            });
        }
        Ok(())
    }

    pub fn check_region_blocks(self, requested: usize) -> Result<(), DecodeLimitError> {
        if requested > self.max_region_blocks {
            return Err(DecodeLimitError::LimitExceeded {
                label: "region blocks",
                requested: requested as u128,
                limit: self.max_region_blocks as u128,
            });
        }
        Ok(())
    }

    pub fn check_vox_models(self, requested: usize) -> Result<(), DecodeLimitError> {
        if requested > self.max_vox_models {
            return Err(DecodeLimitError::LimitExceeded {
                label: "vox models",
                requested: requested as u128,
                limit: self.max_vox_models as u128,
            });
        }
        Ok(())
    }

    pub fn check_vox_total_voxels(self, requested: u64) -> Result<(), DecodeLimitError> {
        if requested > self.max_vox_total_voxels {
            return Err(DecodeLimitError::LimitExceeded {
                label: "vox total voxels",
                requested: requested as u128,
                limit: self.max_vox_total_voxels as u128,
            });
        }
        Ok(())
    }

    pub fn check_vox_nodes(self, requested: usize) -> Result<(), DecodeLimitError> {
        if requested > self.max_vox_nodes {
            return Err(DecodeLimitError::LimitExceeded {
                label: "vox nodes",
                requested: requested as u128,
                limit: self.max_vox_nodes as u128,
            });
        }
        Ok(())
    }

    pub fn check_string_bytes(self, requested: usize) -> Result<(), DecodeLimitError> {
        if requested > self.max_string_bytes {
            return Err(DecodeLimitError::LimitExceeded {
                label: "string bytes",
                requested: requested as u128,
                limit: self.max_string_bytes as u128,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeLimitError {
    LimitExceeded {
        label: &'static str,
        requested: u128,
        limit: u128,
    },
    AllocationFailed {
        label: &'static str,
        requested: usize,
    },
}

impl std::fmt::Display for DecodeLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LimitExceeded {
                label,
                requested,
                limit,
            } => write!(f, "{label} limit exceeded: requested {requested}, limit {limit}"),
            Self::AllocationFailed { label, requested } => {
                write!(f, "{label} allocation failed for {requested} bytes")
            }
        }
    }
}

impl std::error::Error for DecodeLimitError {}

pub fn reserve_vec<T>(
    vec: &mut Vec<T>,
    label: &'static str,
    additional: usize,
) -> Result<(), DecodeLimitError> {
    vec.try_reserve(additional)
        .map_err(|_| DecodeLimitError::AllocationFailed {
            label,
            requested: additional.saturating_mul(std::mem::size_of::<T>()),
        })
}

#[cfg(test)]
mod tests {
    use super::{DecodeLimitError, DecodeLimits};

    #[test]
    fn default_limits_allow_reasonable_small_inputs() {
        let limits = DecodeLimits::default();

        assert!(limits.check_bytes("raw", 1024).is_ok());
        assert!(limits.check_block_voxels(16 * 16 * 16).is_ok());
        assert!(limits.check_vox_models(1).is_ok());
    }

    #[test]
    fn byte_limit_reports_requested_and_limit() {
        let limits = DecodeLimits {
            max_bytes: 4,
            ..DecodeLimits::default()
        };

        assert_eq!(
            limits.check_bytes("payload", 5),
            Err(DecodeLimitError::LimitExceeded {
                label: "payload",
                requested: 5,
                limit: 4
            })
        );
    }
}
```

- [ ] **Step 2: Export module**

Add to `rust/voxel-core/src/streams/mod.rs`:

```rust
pub mod decode_limits;
pub use decode_limits::{DecodeLimitError, DecodeLimits};
```

- [ ] **Step 3: Run focused test**

Run: `cargo test -p voxel-core streams::decode_limits --locked`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add rust/voxel-core/src/streams/decode_limits.rs rust/voxel-core/src/streams/mod.rs
git commit -m "feat(rust): add decode limit policy"
```

### Task 2: Bound Compression Decompression Allocation

**Files:**
- Modify: `rust/voxel-core/src/streams/compressed_data.rs`
- Test: `rust/voxel-core/src/streams/compressed_data.rs`

**Interfaces:**
- Consumes: `DecodeLimits` and `DecodeLimitError`.
- Produces: `pub fn decompress_with_limits(src: &[u8], dst: &mut Vec<u8>, limits: DecodeLimits) -> Result<()>`.

- [ ] **Step 1: Add failing tests**

Add these tests to `compressed_data.rs` tests:

```rust
    #[test]
    fn lz4_decode_rejects_declared_size_over_limit_before_resizing() {
        let mut bytes = vec![Compression::Lz4 as u8];
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 1]);
        let mut dst = Vec::new();
        let limits = crate::streams::DecodeLimits {
            max_bytes: 8,
            ..crate::streams::DecodeLimits::default()
        };

        let err = decompress_with_limits(&bytes, &mut dst, limits).unwrap_err();

        assert!(matches!(err, Error::Limit(_)));
        assert!(dst.is_empty());
    }

    #[test]
    fn none_decode_rejects_payload_over_limit_before_copying() {
        let bytes = vec![Compression::None as u8, 1, 2, 3, 4, 5];
        let mut dst = Vec::new();
        let limits = crate::streams::DecodeLimits {
            max_bytes: 4,
            ..crate::streams::DecodeLimits::default()
        };

        let err = decompress_with_limits(&bytes, &mut dst, limits).unwrap_err();

        assert!(matches!(err, Error::Limit(_)));
        assert!(dst.is_empty());
    }
```

- [ ] **Step 2: Run first failing test**

Run: `cargo test -p voxel-core streams::compressed_data::tests::lz4_decode_rejects_declared_size_over_limit_before_resizing --locked`

Expected: FAIL because `decompress_with_limits` and `Error::Limit` do not exist.

- [ ] **Step 3: Add limit error variant and wrapper**

Add to imports:

```rust
use crate::streams::decode_limits::{reserve_vec, DecodeLimitError, DecodeLimits};
```

Add to `Error`:

```rust
    /// Declared decoded size exceeded caller-provided limits or allocation failed.
    Limit(DecodeLimitError),
```

Add to `Display`:

```rust
            Error::Limit(e) => write!(f, "compressed_data: decode limit: {e}"),
```

Add helper:

```rust
fn prepare_output(dst: &mut Vec<u8>, len: usize, limits: DecodeLimits) -> Result<()> {
    limits.check_bytes("decompressed bytes", len).map_err(Error::Limit)?;
    dst.clear();
    reserve_vec(dst, "decompressed bytes", len).map_err(Error::Limit)?;
    dst.resize(len, 0);
    Ok(())
}
```

Change public decompression entry:

```rust
pub fn decompress(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    decompress_with_limits(src, dst, DecodeLimits::default())
}

pub fn decompress_with_limits(src: &[u8], dst: &mut Vec<u8>, limits: DecodeLimits) -> Result<()> {
    let mut r = MemoryReader::little(src);
    let tag = r.try_get_8().ok_or(Error::UnexpectedEof)?;
    let comp = Compression::from_u8(tag).ok_or(Error::InvalidCompression(tag))?;

    match comp {
        Compression::None => {
            let payload = &src[1..];
            limits
                .check_bytes("uncompressed payload", payload.len())
                .map_err(Error::Limit)?;
            dst.clear();
            reserve_vec(dst, "uncompressed payload", payload.len()).map_err(Error::Limit)?;
            dst.extend_from_slice(payload);
            Ok(())
        }
        Compression::Lz4Be => {
            r.set_endianness(Endianness::BigEndian);
            decompress_lz4(&mut r, src, dst, limits)
        }
        Compression::Lz4 => decompress_lz4(&mut r, src, dst, limits),
        Compression::Zstd => decompress_zstd(&mut r, src, dst, limits),
    }
}
```

Update helper signatures and allocations:

```rust
fn decompress_lz4(
    r: &mut MemoryReader<'_>,
    src: &[u8],
    dst: &mut Vec<u8>,
    limits: DecodeLimits,
) -> Result<()> {
    let decompressed_size = i64::from(r.try_get_32().ok_or(Error::UnexpectedEof)?);
    let decompressed_size = decompressed_size as usize;
    let payload = src.get(SIZE_HEADER_LEN..).ok_or(Error::UnexpectedEof)?;

    prepare_output(dst, decompressed_size, limits)?;
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
```

For `zstd`, use the same signature and call `prepare_output(dst, decompressed_size, limits)?` before decode; after `decode_all`, copy into `dst` as current code does.

- [ ] **Step 4: Run compression tests**

Run: `cargo test -p voxel-core streams::compressed_data --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/streams/compressed_data.rs
git commit -m "fix(rust): bound compressed data decode"
```

### Task 3: Bound Block Serializer Dimensions and Decompressed Bytes

**Files:**
- Modify: `rust/voxel-core/src/streams/block_serializer.rs`
- Test: `rust/voxel-core/src/streams/block_serializer.rs`

**Interfaces:**
- Consumes: `DecodeLimits`, `compressed_data::decompress_with_limits`.
- Produces: `deserialize_with_limits` and `decompress_and_deserialize_with_limits`.

- [ ] **Step 1: Add failing tests**

Add to `block_serializer.rs` tests:

```rust
    fn header_only_block(size: Vector3i) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(BLOCK_FORMAT_VERSION);
        bytes.extend_from_slice(&(size.x as u16).to_le_bytes());
        bytes.extend_from_slice(&(size.y as u16).to_le_bytes());
        bytes.extend_from_slice(&(size.z as u16).to_le_bytes());
        bytes.extend_from_slice(&BLOCK_TRAILING_MAGIC.to_le_bytes());
        bytes
    }

    #[test]
    fn deserialize_rejects_block_voxel_count_over_limit_before_create() {
        let bytes = header_only_block(Vector3i::new(8, 8, 8));
        let limits = crate::streams::DecodeLimits {
            max_block_voxels: 16,
            ..crate::streams::DecodeLimits::default()
        };
        let mut dst = VoxelBuffer::new(Allocator::Default);

        let err = deserialize_with_limits(&bytes, &mut dst, limits).unwrap_err();

        assert!(matches!(err, Error::Limit(_)));
        assert_eq!(dst.size(), Vector3i::zero());
    }
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p voxel-core streams::block_serializer::tests::deserialize_rejects_block_voxel_count_over_limit_before_create --locked`

Expected: FAIL because `deserialize_with_limits` and `Error::Limit` do not exist.

- [ ] **Step 3: Add limit-aware implementation**

Add imports:

```rust
use crate::streams::decode_limits::{DecodeLimitError, DecodeLimits};
```

Add to `Error`:

```rust
    /// Declared block size exceeded caller-provided limits or allocation failed.
    Limit(DecodeLimitError),
```

Add to `Display`:

```rust
            Error::Limit(e) => write!(f, "block_serializer: decode limit: {e}"),
```

Change public wrapper and add bounded entry:

```rust
pub fn deserialize(src: &[u8], buffer: &mut VoxelBuffer) -> Result<(), Error> {
    deserialize_with_limits(src, buffer, DecodeLimits::default())
}

pub fn deserialize_with_limits(
    src: &[u8],
    buffer: &mut VoxelBuffer,
    limits: DecodeLimits,
) -> Result<(), Error> {
```

Inside the bounded function, replace the direct create after reading `size_x/y/z` with:

```rust
    let size = crate::math::Vector3i::new(size_x, size_y, size_z);
    let voxel_count = size.volume_u64();
    limits
        .check_block_voxels(voxel_count)
        .map_err(Error::Limit)?;
    let worst_case_bytes = (voxel_count as usize)
        .checked_mul(MAX_CHANNELS)
        .and_then(|v| v.checked_mul(std::mem::size_of::<u64>()))
        .ok_or_else(|| Error::InvalidFormat("block byte count overflow".to_string()))?;
    limits
        .check_bytes("block voxel bytes", worst_case_bytes)
        .map_err(Error::Limit)?;
    buffer.create(size);
```

Change compressed wrapper:

```rust
pub fn decompress_and_deserialize(src: &[u8], buffer: &mut VoxelBuffer) -> Result<(), Error> {
    decompress_and_deserialize_with_limits(src, buffer, DecodeLimits::default())
}

pub fn decompress_and_deserialize_with_limits(
    src: &[u8],
    buffer: &mut VoxelBuffer,
    limits: DecodeLimits,
) -> Result<(), Error> {
    let mut raw = Vec::new();
    compressed_data::decompress_with_limits(src, &mut raw, limits)?;
    match deserialize_with_limits(&raw, buffer, limits) {
        Ok(()) => Ok(()),
        Err(Error::MetadataSkipped) => Ok(()),
        Err(e) => Err(e),
    }
}
```

- [ ] **Step 4: Run block serializer tests**

Run: `cargo test -p voxel-core streams::block_serializer --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/streams/block_serializer.rs
git commit -m "fix(rust): bound block deserialization"
```

### Task 4: Bound Region Block Payload Loading

**Files:**
- Modify: `rust/voxel-core/src/streams/region/region_file.rs`
- Test: `rust/voxel-core/src/streams/region/region_file.rs`

**Interfaces:**
- Consumes: `DecodeLimits` and `block_serializer::decompress_and_deserialize_with_limits`.
- Produces: `RegionFile::load_block_with_limits`.

- [ ] **Step 1: Add failing test**

Add a test that saves a small block, corrupts the stored length prefix to exceed the sector allocation, then verifies `load_block` returns `BadHeader` or `BlockSerializer` without allocating. Use the existing in-memory `MemoryFile` helper in `region_file.rs` tests. The core assertion must be:

```rust
let err = region.load_block_with_limits(position, &mut loaded, DecodeLimits {
    max_bytes: 4,
    ..DecodeLimits::default()
}).unwrap_err();
assert!(matches!(err, RegionError::BadHeader(_) | RegionError::BlockSerializer(_)));
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p voxel-core streams::region::region_file::tests::load_block_rejects_payload_larger_than_sector_allocation --locked`

Expected: FAIL because `load_block_with_limits` does not exist.

- [ ] **Step 3: Add bounded load API**

Add import:

```rust
use crate::streams::DecodeLimits;
```

Change `load_block` into a wrapper:

```rust
    pub fn load_block(
        &mut self,
        position: Vector3i,
        out_block: &mut VoxelBuffer,
    ) -> Result<(), RegionError> {
        self.load_block_with_limits(position, out_block, DecodeLimits::default())
    }

    pub fn load_block_with_limits(
        &mut self,
        position: Vector3i,
        out_block: &mut VoxelBuffer,
        limits: DecodeLimits,
    ) -> Result<(), RegionError> {
```

Before allocating `payload`, add:

```rust
        let max_payload_in_slot = (bi.sector_count() as usize)
            .checked_mul(self.header.format.sector_size as usize)
            .and_then(|v| v.checked_sub(4))
            .ok_or_else(|| RegionError::BadHeader("invalid block sector allocation".into()))?;
        if block_data_size > max_payload_in_slot {
            return Err(RegionError::BadHeader(format!(
                "block payload length {block_data_size} exceeds sector allocation {max_payload_in_slot}"
            )));
        }
        limits
            .check_bytes("region block payload", block_data_size)
            .map_err(|e| RegionError::BadHeader(e.to_string()))?;
```

Replace payload allocation with fallible reservation:

```rust
        let mut payload = Vec::new();
        payload
            .try_reserve(block_data_size)
            .map_err(|_| RegionError::BadHeader(format!(
                "region block payload allocation failed for {block_data_size} bytes"
            )))?;
        payload.resize(block_data_size, 0);
```

Call:

```rust
        block_serializer::decompress_and_deserialize_with_limits(&payload, out_block, limits)
            .map_err(RegionError::BlockSerializer)?;
```

- [ ] **Step 4: Run region file tests**

Run: `cargo test -p voxel-core streams::region::region_file --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/streams/region/region_file.rs
git commit -m "fix(rust): bound region block loading"
```

### Task 5: Bound VOX Model, Node, String, and Voxel Counts

**Files:**
- Modify: `rust/voxel-core/src/format/vox/parser.rs`
- Modify: `rust/voxel-core/src/format/vox/mod.rs`
- Test: `rust/voxel-core/src/format/vox/tests.rs`

**Interfaces:**
- Consumes: `DecodeLimits`.
- Produces: `pub fn parse_with_limits(bytes: &[u8], limits: DecodeLimits) -> Result<Data>`.

- [ ] **Step 1: Add failing tests**

Add to `format/vox/tests.rs`:

```rust
#[test]
fn parse_with_limits_rejects_dense_model_allocation_over_limit() {
    let mut size = Vec::new();
    size.extend_from_slice(&u32_le(16));
    size.extend_from_slice(&u32_le(16));
    size.extend_from_slice(&u32_le(16));
    let mut xyzi = Vec::new();
    xyzi.extend_from_slice(&u32_le(0));
    let bytes = vox_file(&[(b"SIZE", size), (b"XYZI", xyzi)]);
    let limits = crate::streams::DecodeLimits {
        max_vox_total_voxels: 16,
        ..crate::streams::DecodeLimits::default()
    };

    match super::parse_with_limits(&bytes, limits).unwrap_err() {
        VoxError::InvalidData(message) => assert!(message.contains("vox total voxels")),
        other => panic!("expected InvalidData, got {other:?}"),
    }
}

#[test]
fn parse_with_limits_rejects_too_many_models() {
    let mut chunks = Vec::new();
    for _ in 0..2 {
        let mut size = Vec::new();
        size.extend_from_slice(&u32_le(1));
        size.extend_from_slice(&u32_le(1));
        size.extend_from_slice(&u32_le(1));
        let mut xyzi = Vec::new();
        xyzi.extend_from_slice(&u32_le(0));
        chunks.push((b"SIZE", size));
        chunks.push((b"XYZI", xyzi));
    }
    let bytes = vox_file(&chunks);
    let limits = crate::streams::DecodeLimits {
        max_vox_models: 1,
        ..crate::streams::DecodeLimits::default()
    };

    match super::parse_with_limits(&bytes, limits).unwrap_err() {
        VoxError::InvalidData(message) => assert!(message.contains("vox models")),
        other => panic!("expected InvalidData, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run failing VOX limit test**

Run: `cargo test -p voxel-core format::vox::tests::parse_with_limits_rejects_dense_model_allocation_over_limit --locked`

Expected: FAIL because `parse_with_limits` does not exist.

- [ ] **Step 3: Add bounded parser entry and checks**

Add import:

```rust
use crate::streams::DecodeLimits;
```

Change string parser signature:

```rust
fn parse_string(r: &mut Reader<'_>, limits: DecodeLimits) -> Result<String> {
    let size = i32_from_u32(r.u32()?);
    if size < 0 {
        return Err(VoxError::InvalidData(format!("string length out of range: {size}")));
    }
    limits
        .check_string_bytes(size as usize)
        .map_err(|e| VoxError::InvalidData(e.to_string()))?;
    let bytes = r.take(size as usize)?;
    std::str::from_utf8(bytes)
        .map(|s| s.to_owned())
        .map_err(|_| VoxError::InvalidData("string is not valid UTF-8".into()))
}
```

Thread `limits` through `parse_dictionary` and `parse_node_common_header`.

Change public parse:

```rust
pub fn parse(bytes: &[u8]) -> Result<Data> {
    parse_with_limits(bytes, DecodeLimits::default())
}

pub fn parse_with_limits(bytes: &[u8], limits: DecodeLimits) -> Result<Data> {
```

Maintain counters in `parse_with_limits`:

```rust
    let mut total_dense_voxels = 0u64;
    let mut scene_node_count = 0usize;
```

Before pushing a model in the `XYZI` branch:

```rust
            limits
                .check_vox_models(data.models.len() + 1)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            let dense_voxels = last_size.volume_u64();
            total_dense_voxels = total_dense_voxels
                .checked_add(dense_voxels)
                .ok_or_else(|| VoxError::InvalidData("vox total voxel count overflow".into()))?;
            limits
                .check_vox_total_voxels(total_dense_voxels)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            let num_voxels = r.u32()?;
            if u64::from(num_voxels) > dense_voxels {
                return Err(VoxError::InvalidData(format!(
                    "XYZI voxel count {num_voxels} exceeds model volume {dense_voxels}"
                )));
            }
```

Use fallible reservation for `color_indexes`:

```rust
            let color_index_len = dense_voxels as usize;
            let mut color_indexes = Vec::new();
            color_indexes.try_reserve(color_index_len).map_err(|_| {
                VoxError::InvalidData(format!(
                    "model color index allocation failed for {color_index_len} bytes"
                ))
            })?;
            color_indexes.resize(color_index_len, 0u8);
            let mut model = Model {
                size: last_size,
                color_indexes,
            };
```

In every node branch before inserting into `scene_graph`, increment and check:

```rust
            scene_node_count += 1;
            limits
                .check_vox_nodes(scene_node_count)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
```

Add export in `vox/mod.rs`:

```rust
pub use parser::{parse, parse_with_limits, VoxError};
```

- [ ] **Step 4: Run VOX tests**

Run: `cargo test -p voxel-core format::vox --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/format/vox/parser.rs rust/voxel-core/src/format/vox/mod.rs rust/voxel-core/src/format/vox/tests.rs
git commit -m "fix(rust): bound vox parsing"
```

### Task 6: Full Decode Verification

**Files:**
- Modify: none after previous tasks.
- Test: all decode surfaces.

**Interfaces:**
- Consumes: Tasks 1-5.
- Produces: evidence all untrusted decode surfaces have default and explicit bounded paths.

- [ ] **Step 1: Run focused decode tests**

Run: `cargo test -p voxel-core streams::compressed_data streams::block_serializer streams::region::region_file format::vox --locked`

Expected: PASS.

- [ ] **Step 2: Search for old direct decompression callers**

Run: `rg -n "decompress_and_deserialize\\(|decompress\\(" rust/voxel-core/src`

Expected: existing default wrappers may remain; region load must call `decompress_and_deserialize_with_limits` from its bounded path.

- [ ] **Step 3: Commit if previous tasks were batched**

```bash
git add rust/voxel-core/src/streams/decode_limits.rs rust/voxel-core/src/streams/mod.rs rust/voxel-core/src/streams/compressed_data.rs rust/voxel-core/src/streams/block_serializer.rs rust/voxel-core/src/streams/region/region_file.rs rust/voxel-core/src/format/vox/parser.rs rust/voxel-core/src/format/vox/mod.rs rust/voxel-core/src/format/vox/tests.rs
git commit -m "fix(rust): bound untrusted decode paths"
```
