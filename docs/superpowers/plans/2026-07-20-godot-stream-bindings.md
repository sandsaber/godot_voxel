# Godot Stream Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose memory and persistent region-file voxel streams as Godot Resources, connect them to `VoxelTerrain`, and synchronize the Rust migration documentation with the verified implementation.

**Architecture:** `voxel-gdext` owns thin Resource wrappers that retain typed `Arc` handles and resolve them to `Arc<dyn VoxelStream>` during `VoxelTerrain::_ready`. `voxel-core` owns all persistence behavior: the existing `MemoryStream`, a new thread-safe `RegionFilesStream` forest wrapper, upstream-compatible `meta.vxrm`, and an eight-entry LRU over existing `RegionFile` handles.

**Tech Stack:** Rust 1.96.1 workspace, Godot 4.7 via godot-rust 0.5, `std::sync`, existing voxel-core stream/region APIs, `serde` + `serde_json` for `meta.vxrm`.

## Global Constraints

- Implement in order: `VoxelStreamMemory`, terrain memory-stream wiring, `RegionFilesStream`, then `VoxelStreamRegionFiles`.
- Preserve generator-only behavior when `VoxelTerrain.stream` is unset.
- Stream replacement after `VoxelTerrain::_ready` is outside scope; setters configure the next initialization.
- Keep core code independent of Godot and free of panics for user paths or file contents.
- Preserve upstream names and layout: `meta.vxrm`, `regions/lod{lod}/r.{x}.{y}.{z}.vxr`, metadata version 3.
- Defaults are `block_size_po2 = 4`, `region_size_po2 = 4`, `sector_size = 512`, `max_open_regions = 8`.
- Missing metadata, region files, or blocks produce `LoadResult::NotFound`; corrupt/incompatible content produces `VoxelStreamError`.
- Each task uses red-green-refactor and ends in its own commit.
- Leave unrelated `.zcode/` content untouched.

## File Structure

- `rust/voxel-gdext/src/streams.rs`: Godot stream Resources and Resource-to-core resolution.
- `rust/voxel-gdext/src/terrain.rs`: `stream` property and terrain-core construction policy.
- `rust/voxel-core/src/streams/stream_memory.rs`: supported runtime documentation; existing inspection API remains authoritative.
- `rust/voxel-core/src/streams/region/region_file.rs`: formatted creation entry point used by the forest wrapper.
- `rust/voxel-core/src/streams/stream_region_files.rs`: metadata, position/path mapping, cache, and `VoxelStream` implementation.
- `rust/voxel-core/src/streams/mod.rs`: public stream exports.
- `rust/voxel-core/Cargo.toml`: runtime JSON dependencies.
- `rust/STATUS.md`, `rust/AUDIT.md`, `rust/voxel-gdext/README.md`: final status/API cleanup.

---

### Task 1: Memory Stream Godot Resource

**Files:**
- Create: `rust/voxel-gdext/src/streams.rs`
- Modify: `rust/voxel-gdext/src/lib.rs`
- Modify: `rust/voxel-core/src/streams/stream_memory.rs`

**Interfaces:**
- Consumes: `MemoryStream::{new,len,clear}`, `VoxelStream`.
- Produces: `VoxelStreamMemory`, `MemoryStreamHandle::core_stream()`, `resolve_core_stream()`.

- [ ] **Step 1: Write the failing shared-state test**

Add `mod streams;` to `rust/voxel-gdext/src/lib.rs`, create `streams.rs`, and add this test before defining `MemoryStreamHandle`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use voxel_core::math::Vector3i;
    use voxel_core::storage::VoxelBuffer;

    #[test]
    fn memory_handle_exposes_one_shared_stream() {
        let handle = MemoryStreamHandle::default();
        let core = handle.typed_stream();
        core.save_block(
            Vector3i::new(2, -1, 4),
            0,
            &VoxelBuffer::with_size(Vector3i::splat(1)),
        );

        assert_eq!(handle.block_count(), 1);
        handle.clear();
        assert_eq!(core.len(), 0);
    }
}
```

- [ ] **Step 2: Run the test to verify RED**

Run: `cd rust && cargo test -p voxel-gdext memory_handle_exposes_one_shared_stream`

Expected: compilation fails because `MemoryStreamHandle` is not defined.

- [ ] **Step 3: Implement the handle and Resource**

Add these production types to `streams.rs`:

```rust
use std::sync::Arc;

use godot::prelude::*;
use voxel_core::streams::{MemoryStream, VoxelStream};

#[derive(Clone, Default)]
pub(crate) struct MemoryStreamHandle {
    stream: Arc<MemoryStream>,
}

impl MemoryStreamHandle {
    pub(crate) fn typed_stream(&self) -> Arc<MemoryStream> {
        self.stream.clone()
    }

    pub(crate) fn core_stream(&self) -> Arc<dyn VoxelStream> {
        self.stream.clone()
    }

    fn block_count(&self) -> usize {
        self.stream.len()
    }

    fn clear(&self) {
        self.stream.clear();
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamMemory {
    base: Base<Resource>,
    handle: MemoryStreamHandle,
}

#[godot_api]
impl IResource for VoxelStreamMemory {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            handle: MemoryStreamHandle::default(),
        }
    }
}

#[godot_api]
impl VoxelStreamMemory {
    #[func]
    fn get_block_count(&self) -> i32 {
        i32::try_from(self.handle.block_count()).unwrap_or(i32::MAX)
    }

    #[func]
    fn clear(&self) {
        self.handle.clear();
    }

    pub(crate) fn core_stream(&self) -> Arc<dyn VoxelStream> {
        self.handle.core_stream()
    }
}

pub(crate) fn resolve_core_stream(resource: Gd<Resource>) -> Option<Arc<dyn VoxelStream>> {
    resource
        .clone()
        .try_cast::<VoxelStreamMemory>()
        .ok()
        .map(|stream| stream.bind().core_stream())
}
```

Rewrite the `stream_memory.rs` module and type docs to describe a supported in-memory runtime backend. Remove claims that it is fake or test-only; do not change storage behavior.

- [ ] **Step 4: Verify GREEN and formatting**

Run: `cd rust && cargo test -p voxel-gdext memory_handle_exposes_one_shared_stream && cargo fmt --all -- --check`

Expected: one test passes; fmt is clean.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-gdext/src/streams.rs rust/voxel-gdext/src/lib.rs rust/voxel-core/src/streams/stream_memory.rs
git commit -m "rust(phase5): add VoxelStreamMemory resource"
```

### Task 2: VoxelTerrain Stream Property and Memory Round Trip

**Files:**
- Modify: `rust/voxel-gdext/src/terrain.rs`
- Modify: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`

**Interfaces:**
- Consumes: `resolve_core_stream(Gd<Resource>)`, `VoxelTerrainCore::{new_generator_only,new_with_lod_count}`.
- Produces: Godot `VoxelTerrain.stream`, `select_terrain_stream`; persisted edits reload through the same memory stream.

- [ ] **Step 1: Write the failing selection-policy test and core persistence regression**

Add this test module to `rust/voxel-gdext/src/terrain.rs` before defining `select_terrain_stream`:

```rust
#[cfg(test)]
mod stream_selection_tests {
    use super::*;
    use voxel_core::streams::{MemoryStream, VoxelStream};

    #[test]
    fn explicit_stream_wins_and_only_multi_lod_gets_an_internal_fallback() {
        let explicit: Arc<dyn VoxelStream> = Arc::new(MemoryStream::new());
        let selected = select_terrain_stream(Some(explicit.clone()), 1).unwrap();
        assert!(Arc::ptr_eq(&selected, &explicit));
        assert!(select_terrain_stream(None, 1).is_none());
        assert!(select_terrain_stream(None, 2).is_some());
    }
}
```

Add after `unloading_modified_data_block_saves_it_to_stream`:

```rust
#[test]
fn memory_stream_restores_saved_edit_in_a_new_terrain_core() {
    let stream = Arc::new(MemoryStream::new());
    let mut first = build_core_with_stream(stream.clone());
    let block_size = first.data_block_size();
    let edited_voxel = Vector3i::new(1, 1, 1);
    let channel = ChannelId::Type.index();
    let viewer = [ViewerUpdate {
        id: 1,
        world_position_voxels: Vector3i::zero(),
        horizontal_view_distance_voxels: block_size,
        vertical_view_distance_voxels: block_size,
        requires_meshes: true,
    }];

    process_until(&mut first, &viewer, |core, _| {
        core.data().block_snapshot(Vector3i::zero(), 0).is_some()
    });
    assert!(first.data().try_set_voxel(91, edited_voxel, channel));
    first
        .data()
        .mark_area_modified(Box3i::new(edited_voxel, Vector3i::splat(1)), false);
    process_until(&mut first, &[], |_core, _| stream.len() == 1);
    drop(first);

    let mut second = build_core_with_stream(stream);
    process_until(&mut second, &viewer, |core, _| {
        core.data().block_snapshot(Vector3i::zero(), 0).is_some()
    });
    let restored = second
        .data()
        .block_snapshot(Vector3i::zero(), 0)
        .expect("saved block restored");
    assert_eq!(restored.voxels().get_voxel(1, 1, 1, channel), 91);
}
```

- [ ] **Step 2: Run the test to verify RED**

Run: `cd rust && cargo test -p voxel-gdext explicit_stream_wins_and_only_multi_lod_gets_an_internal_fallback`

Expected: compilation fails because `select_terrain_stream` does not exist.

- [ ] **Step 3: Add the Godot property and construction policy**

Add these fields to `VoxelTerrain` and initialize both with `Default::default()`/`None`:

```rust
#[export]
#[var(get = get_stream, set = set_stream)]
stream: PhantomVar<Option<Gd<Resource>>>,
stream_resource: Option<Gd<Resource>>,
```

The `PhantomVar` registers the Inspector/GDScript property named `stream`; the separate option retains the assigned Resource. Add:

```rust
#[func]
fn get_stream(&self) -> Option<Gd<Resource>> {
    self.stream_resource.clone()
}

#[func]
fn set_stream(&mut self, value: Option<Gd<Resource>>) {
    self.stream_resource = value;
}
```

Define the selection helper:

```rust
fn select_terrain_stream(
    explicit: Option<Arc<dyn voxel_core::streams::VoxelStream>>,
    lod_count: u8,
) -> Option<Arc<dyn voxel_core::streams::VoxelStream>> {
    explicit.or_else(|| {
        (lod_count > 1).then(|| {
            Arc::new(voxel_core::streams::MemoryStream::new())
                as Arc<dyn voxel_core::streams::VoxelStream>
        })
    })
}
```

In `ready`, resolve once and construct the core with this exact policy:

```rust
let stream_was_assigned = self.stream_resource.is_some();
let explicit_stream = self
    .stream_resource
    .clone()
    .and_then(crate::streams::resolve_core_stream);
if stream_was_assigned && explicit_stream.is_none() {
    godot_error!("VoxelTerrain.stream must be VoxelStreamMemory or VoxelStreamRegionFiles");
}
let has_explicit_stream = explicit_stream.is_some();
let selected_stream = select_terrain_stream(explicit_stream, self.lod_count);

let core = match selected_stream {
    Some(stream) => {
        if has_explicit_stream {
            data.set_streaming_enabled(true);
            data.set_full_load_completed(false);
        }
        VoxelTerrainCore::new_with_lod_count(data, stream, meshing_dep, self.lod_count)
    }
    None => VoxelTerrainCore::new_generator_only(data, meshing_dep),
};
```

When `stream_resource` is set but resolution returns `None`, emit one `godot_error!` before using the fallback branch.

- [ ] **Step 4: Verify memory increment**

Run: `cd rust && cargo test -p voxel-core memory_stream_restores_saved_edit_in_a_new_terrain_core && cargo test -p voxel-gdext explicit_stream_wins_and_only_multi_lod_gets_an_internal_fallback && cargo build -p voxel-gdext && cargo clippy --workspace --all-targets && cargo fmt --all -- --check`

Expected: all tests and checks pass.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-gdext/src/terrain.rs rust/voxel-core/src/terrain/voxel_terrain_core.rs
git commit -m "rust(phase5): wire memory streams into VoxelTerrain"
```

### Task 3: Formatted RegionFile Creation

**Files:**
- Modify: `rust/voxel-core/src/streams/region/region_file.rs`

**Interfaces:**
- Consumes: `RegionFormat`, `StdVoxelFile`.
- Produces: `RegionFile::open_with_format(path, create_if_not_found, create_format)`.

- [ ] **Step 1: Write the failing custom-format test**

```rust
#[test]
fn open_with_format_uses_requested_format_for_new_file() {
    let dir = TestDirectory::new().unwrap();
    let path = dir.path().join("custom.vxr");
    let format = RegionFormat {
        region_size: Vector3i::splat(2),
        sector_size: 256,
        ..RegionFormat::default()
    };

    let mut created = RegionFile::open_with_format(&path, true, format.clone()).unwrap();
    created.close().unwrap();
    let reopened = RegionFile::open(&path, false).unwrap();
    assert_eq!(reopened.format(), &format);
}
```

Import the repository's existing `TestDirectory` helper used by neighboring tests.

- [ ] **Step 2: Run the test to verify RED**

Run: `cd rust && cargo test -p voxel-core open_with_format_uses_requested_format_for_new_file`

Expected: compilation fails because `open_with_format` does not exist.

- [ ] **Step 3: Implement formatted creation**

```rust
impl RegionFile<StdVoxelFile> {
    pub fn open(path: &Path, create_if_not_found: bool) -> Result<Self, RegionError> {
        Self::open_with_format(path, create_if_not_found, RegionFormat::default())
    }

    pub fn open_with_format(
        path: &Path,
        create_if_not_found: bool,
        create_format: RegionFormat,
    ) -> Result<Self, RegionError> {
        if !create_format.validate() {
            return Err(RegionError::BadHeader("invalid creation format".into()));
        }
        if path.exists() {
            let mut region = Self::with_format(create_format);
            region.file = Some(StdVoxelFile::open_rw(path).map_err(io)?);
            region.load_header()?;
            Ok(region)
        } else if create_if_not_found {
            let mut region = Self::with_format(create_format);
            region.file = Some(StdVoxelFile::create(path).map_err(io)?);
            region.blocks_begin_offset = region.header.format.header_size_v3() as u64;
            region.save_header()?;
            Ok(region)
        } else {
            Err(RegionError::Io(format!("file not found: {}", path.display())))
        }
    }
}
```

- [ ] **Step 4: Verify GREEN**

Run: `cd rust && cargo test -p voxel-core open_with_format_uses_requested_format_for_new_file && cargo fmt --all -- --check`

Expected: the new test and existing region tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/streams/region/region_file.rs
git commit -m "rust(streams): support formatted region creation"
```

### Task 4: Region Forest Metadata and Mapping

**Files:**
- Create: `rust/voxel-core/src/streams/stream_region_files.rs`
- Modify: `rust/voxel-core/src/streams/mod.rs`
- Modify: `rust/voxel-core/Cargo.toml`
- Modify: `rust/Cargo.lock`

**Interfaces:**
- Produces: `RegionFilesConfig`, `RegionFilesStream::new`, upstream-compatible metadata and paths.
- Consumes later: Task 5 `VoxelStream` implementation; Task 6 Godot Resource.

- [ ] **Step 1: Add runtime JSON dependencies and failing mapping/metadata tests**

Move `serde` and `serde_json` from `[dev-dependencies]` to `[dependencies]`, keeping `serde = { version = "1", features = ["derive"] }` and `serde_json = "1"`.

Create tests in `stream_region_files.rs`:

```rust
#[test]
fn negative_block_positions_map_to_upstream_region_paths() {
    let stream = RegionFilesStream::new("world", RegionFilesConfig::default()).unwrap();
    assert_eq!(
        stream.region_path(Vector3i::new(-1, 0, 16), 2),
        PathBuf::from("world/regions/lod2/r.-1.0.1.vxr")
    );
    assert_eq!(
        stream.split_block_position(Vector3i::new(-1, 0, 16)),
        (Vector3i::new(-1, 0, 1), Vector3i::new(15, 0, 0))
    );
}

#[test]
fn metadata_round_trips_exact_v3_fields() {
    let dir = TestDirectory::new().unwrap();
    let stream = RegionFilesStream::new(dir.path(), RegionFilesConfig::default()).unwrap();
    let mut block = VoxelBuffer::with_size(Vector3i::splat(16));
    block.set_channel_depth(ChannelId::Sdf.index(), ChannelDepth::Bit32);
    stream.initialize_metadata_from(&block).unwrap();

    let json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("meta.vxrm")).unwrap(),
    )
    .unwrap();
    assert_eq!(json["version"], 3);
    assert_eq!(json["block_size_po2"], 4);
    assert_eq!(json["region_size_po2"], 4);
    assert_eq!(json["sector_size"], 512);
    assert_eq!(json["channel_depths"].as_array().unwrap().len(), 8);
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cd rust && cargo test -p voxel-core stream_region_files`

Expected: compilation fails because `RegionFilesStream` and `RegionFilesConfig` do not exist.

- [ ] **Step 3: Implement configuration, metadata, and mapping**

Use these public interfaces and serialized shape:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionFilesConfig {
    pub block_size_po2: u8,
    pub region_size_po2: u8,
    pub sector_size: u32,
    pub max_open_regions: usize,
}

impl Default for RegionFilesConfig {
    fn default() -> Self {
        Self {
            block_size_po2: 4,
            region_size_po2: 4,
            sector_size: 512,
            max_open_regions: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegionMeta {
    version: u8,
    block_size_po2: u8,
    region_size_po2: u8,
    sector_size: u32,
    channel_depths: [u8; MAX_CHANNELS],
}

pub struct RegionFilesStream {
    directory: PathBuf,
    config: RegionFilesConfig,
    state: Mutex<RegionFilesState>,
}

#[derive(Default)]
struct RegionFilesState {
    meta: Option<RegionMeta>,
}

impl RegionFilesStream {
    pub fn new(
        directory: impl Into<PathBuf>,
        config: RegionFilesConfig,
    ) -> StreamResult<Self> {
        validate_config(config)?;
        Ok(Self {
            directory: directory.into(),
            config,
            state: Mutex::new(RegionFilesState::default()),
        })
    }
}
```

Validate block/region powers in `1..=8`, sector size in `256..=65_536`, and `max_open_regions > 0` in `new`. Implement `split_block_position` with signed right shift for the region coordinate and subtraction for the wrapped local coordinate. Implement `region_path` exactly as `regions/lod{lod}/r.{x}.{y}.{z}.vxr`.

`initialize_metadata_from` must create the root directory, atomically write pretty JSON to `meta.vxrm.tmp`, rename it to `meta.vxrm`, and store the parsed metadata in state. Convert `ChannelDepth` with its stable discriminant values; reject values outside the four supported depths when loading.

Export from `streams/mod.rs`:

```rust
pub mod stream_region_files;
pub use stream_region_files::{RegionFilesConfig, RegionFilesStream};
```

- [ ] **Step 4: Verify metadata GREEN**

Run: `cd rust && cargo test -p voxel-core stream_region_files && cargo clippy -p voxel-core --all-targets && cargo fmt --all -- --check`

Expected: mapping and metadata tests pass; checks are clean.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/Cargo.toml rust/Cargo.lock rust/voxel-core/src/streams/mod.rs rust/voxel-core/src/streams/stream_region_files.rs
git commit -m "rust(streams): add region forest metadata"
```

### Task 5: RegionFilesStream Persistence and LRU

**Files:**
- Modify: `rust/voxel-core/src/streams/stream_region_files.rs`

**Interfaces:**
- Consumes: `RegionFile::open_with_format`, `RegionMeta`, `VoxelStream`.
- Produces: filesystem-backed load/save/flush, eight-entry LRU.

- [ ] **Step 1: Add failing persistence tests**

```rust
#[test]
fn saves_reopens_and_loads_negative_positions_per_lod() {
    let dir = TestDirectory::new().unwrap();
    let config = RegionFilesConfig::default();
    let position = Vector3i::new(-1, 0, 16);
    let mut source = VoxelBuffer::with_size(Vector3i::splat(16));
    source.set_voxel(37, 1, 2, 3, ChannelId::Type.index());
    {
        let stream = RegionFilesStream::new(dir.path(), config).unwrap();
        stream
            .save_voxel_block(VoxelSaveQuery::new(&source, position, 2))
            .unwrap();
        stream.flush().unwrap();
    }

    let reopened = RegionFilesStream::new(dir.path(), config).unwrap();
    let mut loaded = VoxelBuffer::new(Allocator::Default);
    assert_eq!(
        reopened
            .load_voxel_block(VoxelLoadQuery::new(&mut loaded, position, 2))
            .unwrap(),
        LoadResult::Found
    );
    assert_eq!(loaded.get_voxel(1, 2, 3, ChannelId::Type.index()), 37);
    assert_eq!(
        reopened
            .load_voxel_block(VoxelLoadQuery::new(&mut loaded, position, 1))
            .unwrap(),
        LoadResult::NotFound
    );
}

#[test]
fn lru_never_exceeds_configured_open_region_limit() {
    let dir = TestDirectory::new().unwrap();
    let config = RegionFilesConfig {
        max_open_regions: 2,
        ..RegionFilesConfig::default()
    };
    let stream = RegionFilesStream::new(dir.path(), config).unwrap();
    let block = VoxelBuffer::with_size(Vector3i::splat(16));
    for x in [0, 16, 32] {
        stream
            .save_voxel_block(VoxelSaveQuery::new(&block, Vector3i::new(x, 0, 0), 0))
            .unwrap();
        assert!(stream.cached_region_count() <= 2);
    }
}
```

Also add tests for missing metadata returning `NotFound`, malformed metadata returning `CorruptData`, and a first block with incompatible size returning `BlockFormatMismatch`.

- [ ] **Step 2: Run tests to verify RED**

Run: `cd rust && cargo test -p voxel-core stream_region_files`

Expected: tests fail because `VoxelStream`, cache, and flush behavior are not implemented.

- [ ] **Step 3: Implement cache and stream contract**

Add `CachedRegion` and extend the Task 4 state with cache and access-clock fields:

```rust
struct CachedRegion {
    position: Vector3i,
    lod: u8,
    last_used: u64,
    file: RegionFile,
}

struct RegionFilesState {
    meta: Option<RegionMeta>,
    cache: Vec<CachedRegion>,
    access_clock: u64,
}
```

Implement `open_region` under the state mutex: return an existing `(position,lod)` entry after updating `last_used`; otherwise evict the minimum `last_used` entry with `close()`, create parent directories for save, build `RegionFormat` from metadata, and call `RegionFile::open_with_format`. Pre-check `path.exists()` for load so missing files map to `NotFound` rather than `Io`.

Implement these trait methods:

```rust
impl VoxelStream for RegionFilesStream {
    fn load_voxel_block(&self, query: VoxelLoadQuery<'_>) -> StreamResult<LoadResult>;
    fn save_voxel_block(&self, query: VoxelSaveQuery<'_>) -> StreamResult<()>;
    fn flush(&self) -> StreamResult<()>;
    fn get_used_channels_mask(&self) -> u8 {
        ALL_CHANNELS_MASK
    }
    fn get_lod_count(&self) -> u8 {
        MAX_LOD as u8
    }
    fn get_supported_save_mode(&self) -> SaveMode {
        SaveMode::Filesystem
    }
}
```

Map `RegionError::BlockNotFound` to `Ok(LoadResult::NotFound)`, format mismatches to `VoxelStreamError::BlockFormatMismatch`, bad headers/serializer failures to `CorruptData`, and filesystem failures to `Io` including the path. Make `cached_region_count` available only under `#[cfg(test)]`. Implement `Drop` with best-effort `flush` and no panic.

- [ ] **Step 4: Verify persistence GREEN**

Run: `cd rust && cargo test -p voxel-core stream_region_files && cargo test -p voxel-core region_file && cargo clippy -p voxel-core --all-targets && cargo fmt --all -- --check`

Expected: all forest and lower-level region tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/streams/stream_region_files.rs
git commit -m "rust(streams): implement region forest persistence"
```

### Task 6: VoxelStreamRegionFiles Godot Resource

**Files:**
- Modify: `rust/voxel-gdext/src/streams.rs`

**Interfaces:**
- Consumes: `RegionFilesConfig`, `RegionFilesStream`, `resolve_core_stream`.
- Produces: Godot `VoxelStreamRegionFiles` Resource and `flush() -> bool`.

- [ ] **Step 1: Write failing pure configuration tests**

```rust
#[test]
fn region_resource_properties_validate_core_ranges() {
    assert!(region_config(4, 4, 512).is_ok());
    assert!(region_config(0, 4, 512).is_err());
    assert!(region_config(4, 9, 512).is_err());
    assert!(region_config(4, 4, 128).is_err());
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cd rust && cargo test -p voxel-gdext region_resource_properties_validate_core_ranges`

Expected: compilation fails because `region_config` does not exist.

- [ ] **Step 3: Implement the Resource and resolver branch**

Add a pure `region_config(block, region, sector) -> Result<RegionFilesConfig, String>` enforcing the core ranges. Register the four inspector properties through `PhantomVar` fields and retain their Rust values separately:

```rust
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamRegionFiles {
    base: Base<Resource>,
    #[export]
    #[var(get = get_directory, set = set_directory)]
    directory: PhantomVar<GString>,
    #[export]
    #[var(get = get_block_size_po2, set = set_block_size_po2)]
    block_size_po2: PhantomVar<i64>,
    #[export]
    #[var(get = get_region_size_po2, set = set_region_size_po2)]
    region_size_po2: PhantomVar<i64>,
    #[export]
    #[var(get = get_sector_size, set = set_sector_size)]
    sector_size: PhantomVar<i64>,
    directory_value: GString,
    block_size_po2_value: i64,
    region_size_po2_value: i64,
    sector_size_value: i64,
    core: Option<Arc<RegionFilesStream>>,
}
```

Initialize phantom fields with `PhantomVar::default()`, values with `GString::new()`, 4, 4, and 512, and `core` with `None`. Each getter returns the corresponding value. Each setter first rejects `core.is_some()`, then validates its individual range and stores the value; invalid input logs `godot_error!` and leaves the old value unchanged.

Implement `core_stream(&mut self) -> Result<Arc<dyn VoxelStream>, String>` so the first call validates properties, creates one `RegionFilesStream`, caches the `Arc`, and subsequent calls return the same Arc. Setters must reject changes once `core.is_some()` and log one `godot_error!`.

Expose:

```rust
#[func]
fn flush(&self) -> bool {
    let Some(stream) = &self.core else {
        return true;
    };
    match stream.flush() {
        Ok(()) => true,
        Err(error) => {
            godot_error!("VoxelStreamRegionFiles flush failed: {error}");
            false
        }
    }
}
```

Extend `resolve_core_stream` with a `try_cast::<VoxelStreamRegionFiles>()` branch using `bind_mut().core_stream()`. Log construction errors and return `None`.

- [ ] **Step 4: Verify Godot binding GREEN**

Run: `cd rust && cargo test -p voxel-gdext && cargo build -p voxel-gdext && cargo clippy --workspace --all-targets && cargo fmt --all -- --check`

Expected: config test passes; binding builds cleanly.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-gdext/src/streams.rs
git commit -m "rust(phase5): add VoxelStreamRegionFiles resource"
```

### Task 7: Status, README, and Full Verification

**Files:**
- Modify: `rust/STATUS.md`
- Modify: `rust/AUDIT.md`
- Modify: `rust/voxel-gdext/README.md`

**Interfaces:**
- Consumes: final verified test counts and Godot API.
- Produces: internally consistent M3.18 documentation.

- [ ] **Step 1: Capture actual verification counts**

Run: `cd rust && cargo test -p voxel-core && cargo test -p voxel-gdext`

Expected before new tests: 707 core unit tests, 11 integration tests, one doc-test. Record the new actual counts from this run; do not retain the stale 655 total.

- [ ] **Step 2: Update documentation**

Make these exact content changes:

- In `STATUS.md`, make the At-a-glance test count and Total line agree with the captured output; add memory and region-file stream Resources to M3; replace “Далее: stream binding” with the next remaining Phase 5 subsystem.
- In `AUDIT.md` §9.7/M3 entry and §11.2 item 18, record both stream Resources, core region forest, save/load/flush, and the verification commands. Keep historical findings unchanged.
- Replace the README's “Phase 2 skeleton” and `VoxelRustHello` example with a current Godot 4.7 example that creates `VoxelTerrain`, assigns a generator and either `VoxelStreamMemory` or `VoxelStreamRegionFiles`, sets `directory`, and calls `flush()` for filesystem persistence.

- [ ] **Step 3: Run stale-text and diff checks**

Run: `rg -n "Phase 2 skeleton|VoxelRustHello|655 unit|stream binding \(save/load\).*⏳" rust/STATUS.md rust/AUDIT.md rust/voxel-gdext/README.md`

Expected: no matches representing current status. Historical numeric entries in the audit log may remain when tied to dated verification rows.

Run: `git diff --check`

Expected: no whitespace errors.

- [ ] **Step 4: Run the complete verification suite**

Run: `cd rust && cargo test -p voxel-core && cargo test -p voxel-gdext && cargo build -p voxel-gdext && cargo clippy --workspace --all-targets && cargo fmt --all -- --check`

Expected: all tests pass, GDExtension builds, clippy is warning-free, and fmt is clean.

- [ ] **Step 5: Commit documentation cleanup**

```bash
git add rust/STATUS.md rust/AUDIT.md rust/voxel-gdext/README.md
git commit -m "docs(audit+status): record M3 stream bindings"
```

- [ ] **Step 6: Confirm final working tree scope**

Run: `git status --short`

Expected: only pre-existing unrelated `.zcode/` remains untracked; all stream-binding files are committed.
