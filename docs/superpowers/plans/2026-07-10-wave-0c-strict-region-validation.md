# Wave 0C Strict Region Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make region format/header/LUT validation strict and fallible before allocation or sector-map rebuild.

**Architecture:** Add explicit format and block-info validation helpers in `region::format`, then call them from constructors and header load before allocating the LUT. Header loading must reject invalid channel depth bytes, zero/oversized region axes, header-size overflows, LUT sector overlaps, out-of-range sector intervals, and block-info setter overflow.

**Tech Stack:** Rust, `voxel-core`, in-memory region file tests.

## Global Constraints

- Preserve existing region file API where possible; add fallible alternatives instead of silently changing every caller.
- Invalid on-disk headers must return `RegionError::BadHeader`.
- In-memory `RegionBlockInfo` setters must reject overflow instead of masking wire values.
- Do not implement region compaction WAL or dual-header persistence in Wave 0C.
- Do not touch `rust/AUDIT.md`; it is user-owned working-tree state.

---

## File Structure

- Modify: `rust/voxel-core/src/streams/region/format.rs`
  - Add `RegionFormatError`.
  - Add `RegionFormat::validate_result`, `RegionFormat::block_count_checked`, `RegionFormat::header_size_v3_checked`.
  - Add `RegionBlockInfo::try_new`, `try_set_sector_index`, `try_set_sector_count`.
- Modify: `rust/voxel-core/src/streams/region/region_file.rs`
  - Validate format immediately after reading header fields and before LUT allocation.
  - Validate LUT sector intervals after reading LUT and before `rebuild_sectors`.
  - Use fallible block-info setters in save/compaction paths.
- Test: existing `format.rs` and `region_file.rs` test modules.

### Task 1: Add Strict RegionFormat and RegionBlockInfo Validation APIs

**Files:**
- Modify: `rust/voxel-core/src/streams/region/format.rs`
- Test: `rust/voxel-core/src/streams/region/format.rs`

**Interfaces:**
- Produces: `RegionFormatError`, `RegionFormat::validate_result`, `block_count_checked`, `header_size_v3_checked`, `RegionBlockInfo::try_*`.
- Consumers: `RegionFile::with_format`, `RegionFile::load_header`, compaction/save code.

- [ ] **Step 1: Add failing format tests**

Add tests to `format.rs`:

```rust
    #[test]
    fn format_rejects_zero_region_axis() {
        let mut f = RegionFormat::default();
        f.region_size = Vector3i::new(0, 16, 16);

        assert!(f.validate_result().is_err());
        assert!(!f.validate());
    }

    #[test]
    fn block_info_try_new_rejects_overflow_without_masking() {
        assert!(RegionBlockInfo::try_new(MAX_SECTOR_INDEX + 1, 1).is_err());
        assert!(RegionBlockInfo::try_new(1, MAX_SECTOR_COUNT + 1).is_err());
    }

    #[test]
    fn block_info_try_setters_reject_overflow_without_changing_value() {
        let mut info = RegionBlockInfo::new(7, 8);

        assert!(info.try_set_sector_index(MAX_SECTOR_INDEX + 1).is_err());
        assert_eq!(info.sector_index(), 7);
        assert!(info.try_set_sector_count(MAX_SECTOR_COUNT + 1).is_err());
        assert_eq!(info.sector_count(), 8);
    }
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p voxel-core streams::region::format::tests::format_rejects_zero_region_axis --locked`

Expected: FAIL because `validate_result` does not exist and current `validate` allows zero axes.

- [ ] **Step 3: Add validation types and helpers**

Add after constants:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionFormatError {
    InvalidRegionAxis { axis: &'static str, value: i32 },
    InvalidBlockSizePo2(u8),
    InvalidSectorSize(u32),
    ByteCountOverflow,
    SectorCountOverflow { sectors_per_block: u64 },
    SectorIndexOverflow { max_potential_sectors: u64 },
    HeaderSizeOverflow,
    RegionBlockInfoOverflow {
        field: &'static str,
        value: u32,
        max: u32,
    },
}

impl std::fmt::Display for RegionFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRegionAxis { axis, value } => {
                write!(f, "invalid region {axis} axis {value}")
            }
            Self::InvalidBlockSizePo2(v) => write!(f, "invalid block_size_po2 {v}"),
            Self::InvalidSectorSize(v) => write!(f, "invalid sector_size {v}"),
            Self::ByteCountOverflow => write!(f, "region byte count overflow"),
            Self::SectorCountOverflow { sectors_per_block } => {
                write!(f, "sectors per block {sectors_per_block} exceeds {MAX_SECTOR_COUNT}")
            }
            Self::SectorIndexOverflow { max_potential_sectors } => {
                write!(f, "potential sectors {max_potential_sectors} exceeds {MAX_SECTOR_INDEX}")
            }
            Self::HeaderSizeOverflow => write!(f, "region header size overflow"),
            Self::RegionBlockInfoOverflow { field, value, max } => {
                write!(f, "region block info {field} {value} exceeds {max}")
            }
        }
    }
}

impl std::error::Error for RegionFormatError {}
```

Add to `RegionFormat`:

```rust
    pub fn validate_result(&self) -> Result<(), RegionFormatError> {
        for (axis, value) in [
            ("x", self.region_size.x),
            ("y", self.region_size.y),
            ("z", self.region_size.z),
        ] {
            if value <= 0 || value as u32 >= MAX_BLOCKS_ACROSS {
                return Err(RegionFormatError::InvalidRegionAxis { axis, value });
            }
        }
        if self.block_size_po2 == 0 {
            return Err(RegionFormatError::InvalidBlockSizePo2(self.block_size_po2));
        }
        if self.sector_size == 0 {
            return Err(RegionFormatError::InvalidSectorSize(self.sector_size));
        }
        let shift = 3u32
            .checked_mul(self.block_size_po2 as u32)
            .ok_or(RegionFormatError::ByteCountOverflow)?;
        let voxels_per_block = 1u64
            .checked_shl(shift)
            .ok_or(RegionFormatError::ByteCountOverflow)?;
        let mut bytes_per_block = 0u64;
        for d in &self.channel_depths {
            let bytes = (d.bit_count() / 8) as u64;
            bytes_per_block = bytes_per_block
                .checked_add(bytes.checked_mul(voxels_per_block).ok_or(RegionFormatError::ByteCountOverflow)?)
                .ok_or(RegionFormatError::ByteCountOverflow)?;
        }
        let sectors_per_block = bytes_per_block.div_ceil(self.sector_size as u64);
        if sectors_per_block > MAX_SECTOR_COUNT as u64 {
            return Err(RegionFormatError::SectorCountOverflow { sectors_per_block });
        }
        let block_count = self.block_count_checked()? as u64;
        let max_potential_sectors = block_count
            .checked_mul(sectors_per_block)
            .ok_or(RegionFormatError::SectorIndexOverflow {
                max_potential_sectors: u64::MAX,
            })?;
        if max_potential_sectors > MAX_SECTOR_INDEX as u64 {
            return Err(RegionFormatError::SectorIndexOverflow { max_potential_sectors });
        }
        let _ = self.header_size_v3_checked()?;
        Ok(())
    }

    pub fn block_count_checked(&self) -> Result<usize, RegionFormatError> {
        let x = usize::try_from(self.region_size.x)
            .map_err(|_| RegionFormatError::InvalidRegionAxis { axis: "x", value: self.region_size.x })?;
        let y = usize::try_from(self.region_size.y)
            .map_err(|_| RegionFormatError::InvalidRegionAxis { axis: "y", value: self.region_size.y })?;
        let z = usize::try_from(self.region_size.z)
            .map_err(|_| RegionFormatError::InvalidRegionAxis { axis: "z", value: self.region_size.z })?;
        x.checked_mul(y)
            .and_then(|v| v.checked_mul(z))
            .ok_or(RegionFormatError::HeaderSizeOverflow)
    }

    pub fn header_size_v3_checked(&self) -> Result<usize, RegionFormatError> {
        let palette_bytes = if self.palette.is_some() {
            PALETTE_SIZE_IN_BYTES
        } else {
            0
        };
        MAGIC_AND_VERSION_SIZE
            .checked_add(FIXED_HEADER_DATA_SIZE)
            .and_then(|v| v.checked_add(palette_bytes))
            .and_then(|v| {
                self.block_count_checked()
                    .ok()
                    .and_then(|count| count.checked_mul(std::mem::size_of::<RegionBlockInfo>()))
                    .and_then(|lut| v.checked_add(lut))
            })
            .ok_or(RegionFormatError::HeaderSizeOverflow)
    }
```

Change `validate` to:

```rust
    pub fn validate(&self) -> bool {
        self.validate_result().is_ok()
    }
```

Add to `RegionBlockInfo`:

```rust
    pub fn try_new(sector_index: u32, sector_count: u32) -> Result<Self, RegionFormatError> {
        if sector_index > MAX_SECTOR_INDEX {
            return Err(RegionFormatError::RegionBlockInfoOverflow {
                field: "sector_index",
                value: sector_index,
                max: MAX_SECTOR_INDEX,
            });
        }
        if sector_count > MAX_SECTOR_COUNT {
            return Err(RegionFormatError::RegionBlockInfoOverflow {
                field: "sector_count",
                value: sector_count,
                max: MAX_SECTOR_COUNT,
            });
        }
        Ok(Self {
            data: (sector_index << 8) | sector_count,
        })
    }

    pub fn try_set_sector_index(&mut self, i: u32) -> Result<(), RegionFormatError> {
        if i > MAX_SECTOR_INDEX {
            return Err(RegionFormatError::RegionBlockInfoOverflow {
                field: "sector_index",
                value: i,
                max: MAX_SECTOR_INDEX,
            });
        }
        self.data = (i << 8) | (self.data & 0xff);
        Ok(())
    }

    pub fn try_set_sector_count(&mut self, c: u32) -> Result<(), RegionFormatError> {
        if c > MAX_SECTOR_COUNT {
            return Err(RegionFormatError::RegionBlockInfoOverflow {
                field: "sector_count",
                value: c,
                max: MAX_SECTOR_COUNT,
            });
        }
        self.data = c | (self.data & 0xffffff00);
        Ok(())
    }
```

Make existing `new`, `set_sector_index`, and `set_sector_count` call their fallible versions and `expect("validated region block info")` to preserve existing infallible API for internal trusted code.

- [ ] **Step 4: Run format tests**

Run: `cargo test -p voxel-core streams::region::format --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/streams/region/format.rs
git commit -m "fix(rust): validate region format strictly"
```

### Task 2: Validate Region Header Before LUT Allocation

**Files:**
- Modify: `rust/voxel-core/src/streams/region/region_file.rs`
- Test: `rust/voxel-core/src/streams/region/region_file.rs`

**Interfaces:**
- Consumes: `RegionFormat::validate_result`, `block_count_checked`, `header_size_v3_checked`.
- Produces: `load_header` rejects invalid header fields before allocating `blocks`.

- [ ] **Step 1: Add failing invalid header tests**

Add tests using existing in-memory file helpers:

```rust
#[test]
fn open_rejects_region_header_with_zero_axis_before_lut_allocation() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.push(FORMAT_VERSION);
    bytes.push(4);
    bytes.extend_from_slice(&[0, 16, 16]);
    bytes.extend_from_slice(&[ChannelDepth::Bit8 as u8; MAX_CHANNELS]);
    bytes.extend_from_slice(&512u16.to_le_bytes());
    bytes.push(0x00);
    let file = MemoryFile::from_bytes(bytes);

    let err = RegionFile::<MemoryFile>::open(file).unwrap_err();

    assert!(matches!(err, RegionError::BadHeader(message) if message.contains("invalid region x axis")));
}

#[test]
fn open_rejects_region_header_with_invalid_channel_depth() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.push(FORMAT_VERSION);
    bytes.push(4);
    bytes.extend_from_slice(&[16, 16, 16]);
    bytes.extend_from_slice(&[0xff; MAX_CHANNELS]);
    bytes.extend_from_slice(&512u16.to_le_bytes());
    bytes.push(0x00);

    let err = RegionFile::<MemoryFile>::open(MemoryFile::from_bytes(bytes)).unwrap_err();

    assert!(matches!(err, RegionError::BadHeader(message) if message.contains("channel depth")));
}
```

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p voxel-core streams::region::region_file::tests::open_rejects_region_header_with_zero_axis_before_lut_allocation --locked`

Expected: FAIL because load header currently accepts zero axes and invalid depths are discarded.

- [ ] **Step 3: Parse channel depth strictly**

In `load_header`, replace `ChannelDepth::from_u8_discard_invalid(fixed[o])` with a strict match:

```rust
            *d = match fixed[o] {
                0 => ChannelDepth::Bit8,
                1 => ChannelDepth::Bit16,
                2 => ChannelDepth::Bit32,
                3 => ChannelDepth::Bit64,
                other => {
                    return Err(RegionError::BadHeader(format!(
                        "invalid channel depth byte {other:#x}"
                    )));
                }
            };
```

After palette parsing and before LUT allocation:

```rust
        self.header
            .format
            .validate_result()
            .map_err(|e| RegionError::BadHeader(e.to_string()))?;
        let block_count = self
            .header
            .format
            .block_count_checked()
            .map_err(|e| RegionError::BadHeader(e.to_string()))?;
        let lut_bytes = block_count
            .checked_mul(std::mem::size_of::<RegionBlockInfo>())
            .ok_or_else(|| RegionError::BadHeader("region LUT size overflow".into()))?;
        let expected_header_size = self
            .header
            .format
            .header_size_v3_checked()
            .map_err(|e| RegionError::BadHeader(e.to_string()))?;
        if file_len < expected_header_size as u64 {
            return Err(RegionError::BadHeader("truncated block LUT".into()));
        }
```

Remove the old `let block_count = self.header.format.region_size.volume_u64() as usize;` line.

- [ ] **Step 4: Use checked header sizes in constructors and save path**

In `RegionFile::with_format`, call:

```rust
        format
            .validate_result()
            .expect("RegionFile::with_format requires a valid region format");
        let block_count = format
            .block_count_checked()
            .expect("validated region format has checked block count");
```

In `save_header`, use:

```rust
        let header_size = self
            .header
            .format
            .header_size_v3_checked()
            .map_err(|e| RegionError::BadHeader(e.to_string()))?;
        let mut buf: Vec<u8> = Vec::with_capacity(header_size);
```

Set `blocks_begin_offset` with checked size:

```rust
        self.blocks_begin_offset = self
            .header
            .format
            .header_size_v3_checked()
            .map_err(|e| RegionError::BadHeader(e.to_string()))? as u64;
```

- [ ] **Step 5: Run region file tests**

Run: `cargo test -p voxel-core streams::region::region_file --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add rust/voxel-core/src/streams/region/region_file.rs
git commit -m "fix(rust): reject invalid region headers"
```

### Task 3: Validate Region LUT Sector Intervals

**Files:**
- Modify: `rust/voxel-core/src/streams/region/region_file.rs`
- Test: `rust/voxel-core/src/streams/region/region_file.rs`

**Interfaces:**
- Consumes: parsed `Header.blocks`, `blocks_begin_offset`, file length, sector size.
- Produces: `load_header` rejects LUT entries with out-of-file ranges or overlapping sectors.

- [ ] **Step 1: Add failing LUT tests**

Add tests that build a small valid header and inject bad LUT values:

```rust
#[test]
fn open_rejects_region_lut_sector_outside_file() {
    let mut region = RegionFile::<MemoryFile>::with_format(RegionFormat {
        region_size: Vector3i::new(1, 1, 1),
        ..RegionFormat::default()
    });
    let file = MemoryFile::new();
    region.create(file).unwrap();
    let mut bytes = region.into_inner().into_bytes();
    let lut_offset = RegionFormat {
        region_size: Vector3i::new(1, 1, 1),
        ..RegionFormat::default()
    }
    .header_size_v3_checked()
    .unwrap()
        - 4;
    bytes[lut_offset..lut_offset + 4].copy_from_slice(&RegionBlockInfo::new(10, 1).data.to_le_bytes());

    let err = RegionFile::<MemoryFile>::open(MemoryFile::from_bytes(bytes)).unwrap_err();

    assert!(matches!(err, RegionError::BadHeader(message) if message.contains("outside file")));
}
```

If `RegionFile` lacks `into_inner`, add a test-only accessor:

```rust
#[cfg(test)]
fn into_inner(mut self) -> F {
    self.file.take().expect("file open")
}
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p voxel-core streams::region::region_file::tests::open_rejects_region_lut_sector_outside_file --locked`

Expected: FAIL because LUT ranges are not validated before `rebuild_sectors`.

- [ ] **Step 3: Add LUT validator**

Add private helper:

```rust
    fn validate_lut(&self, file_len: u64) -> Result<(), RegionError> {
        let sector_size = self.header.format.sector_size as u64;
        let data_len = file_len.saturating_sub(self.blocks_begin_offset);
        let sector_capacity = data_len.div_ceil(sector_size);
        let mut occupied: Vec<(u32, u32, Vector3i)> = Vec::new();

        for (i, bi) in self.header.blocks.iter().copied().enumerate() {
            if !bi.is_present() {
                continue;
            }
            if bi.sector_count() == 0 {
                return Err(RegionError::BadHeader("present LUT entry has zero sectors".into()));
            }
            let start = bi.sector_index();
            let end = start
                .checked_add(bi.sector_count())
                .ok_or_else(|| RegionError::BadHeader("LUT sector interval overflow".into()))?;
            if end as u64 > sector_capacity {
                return Err(RegionError::BadHeader(format!(
                    "LUT sector interval {start}..{end} outside file sector capacity {sector_capacity}"
                )));
            }
            occupied.push((start, end, self.block_position_from_index(i as u32)));
        }

        occupied.sort_by_key(|(start, _, _)| *start);
        for pair in occupied.windows(2) {
            let (_, prev_end, prev_pos) = pair[0];
            let (next_start, _, next_pos) = pair[1];
            if next_start < prev_end {
                return Err(RegionError::BadHeader(format!(
                    "LUT sectors overlap between {prev_pos:?} and {next_pos:?}"
                )));
            }
        }
        Ok(())
    }
```

Call after assigning `self.header.blocks` and before `rebuild_sectors()`:

```rust
        self.validate_lut(file_len)?;
```

- [ ] **Step 4: Replace mutating setters with fallible variants in region_file**

In `remove_sectors_from_block` and `save_block`, replace direct setters where values are derived from file state:

```rust
        bi.try_set_sector_count(old_count - count)
            .map_err(|e| RegionError::BadHeader(e.to_string()))?;
```

```rust
                other
                    .try_set_sector_index(other.sector_index() - count)
                    .map_err(|e| RegionError::BadHeader(e.to_string()))?;
```

Replace `RegionBlockInfo::new(...)` with:

```rust
            self.header.blocks[lut_index] =
                RegionBlockInfo::try_new(sector_index, new_sector_count)
                    .map_err(|e| RegionError::BadHeader(e.to_string()))?;
```

- [ ] **Step 5: Run region tests**

Run: `cargo test -p voxel-core streams::region --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add rust/voxel-core/src/streams/region/region_file.rs
git commit -m "fix(rust): validate region lut intervals"
```

### Task 4: Strict Region Verification

**Files:**
- Modify: none after previous tasks.
- Test: region-focused tests.

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: evidence strict region validation closes audit Region-1 unsafe inputs.

- [ ] **Step 1: Run all region tests**

Run: `cargo test -p voxel-core streams::region --locked`

Expected: PASS.

- [ ] **Step 2: Search for lossy invalid depth parsing**

Run: `rg -n "from_u8_discard_invalid|set_sector_count\\(|set_sector_index\\(|RegionBlockInfo::new" rust/voxel-core/src/streams/region`

Expected: no header-load use of `from_u8_discard_invalid`; trusted constructors may remain only where values are prevalidated or tests assert infallible behavior.

- [ ] **Step 3: Commit if previous tasks were batched**

```bash
git add rust/voxel-core/src/streams/region/format.rs rust/voxel-core/src/streams/region/region_file.rs
git commit -m "fix(rust): enforce strict region validation"
```
