# Godot Stream Bindings Design

**Goal:** Complete M3.18 in two independently usable increments: expose the existing in-memory stream to Godot first, then add persistent region-file storage compatible with the upstream `VoxelStreamRegionFiles` layout.

**Scope:** This work adds stream Resources, connects them to `VoxelTerrain`, ports the missing region-forest wrapper over the existing `.vxr` implementation, and brings Rust status documentation in line with the verified baseline. It does not add SQLite, instance-block persistence, editor plugins, or runtime stream replacement after `VoxelTerrain::_ready`.

## Architecture

The Godot layer exposes two concrete `Resource` classes:

- `VoxelStreamMemory` owns one `Arc<MemoryStream>` and returns it as `Arc<dyn VoxelStream>`.
- `VoxelStreamRegionFiles` owns one shared core `RegionFilesStream` configured from its exported properties and returns it through the same trait-object boundary.

`VoxelTerrain` gains a `stream` Resource property. During `_ready`, it resolves the assigned Resource to a supported stream class before constructing `VoxelTerrainCore`. The resolved stream is retained by both the Resource and terrain core, so its stored data survives terrain paging and remains inspectable from Godot. With no stream assigned, single-LOD terrain keeps the current generator-only behavior. Multi-LOD terrain continues to use an internal memory stream only when no explicit stream was assigned, preserving existing behavior without exposing a temporary Resource.

Unsupported Resource types are rejected with a Godot error and fall back to the same no-explicit-stream behavior. Stream configuration is fixed after `_ready`; changing the Resource property affects the next initialization rather than mutating dependencies under active worker tasks.

## Increment 1: Memory Stream

Add `rust/voxel-gdext/src/streams.rs` with `VoxelStreamMemory`, registered as a tool-enabled Godot `Resource`. It exposes:

- `get_block_count() -> i32`, reporting the number of stored `(block_position, lod)` entries.
- `clear()`, removing all stored blocks.
- An internal `core_stream() -> Arc<MemoryStream>` used by `VoxelTerrain`.

The existing `MemoryStream` gains only the thread-safe inspection operations needed by the binding. Its save/load contract and key format remain unchanged.

`VoxelTerrain` gains `get_stream() -> Variant` and `set_stream(Gd<Resource>)`. Core construction uses the selected stream for paging. The existing save-on-unload path remains the single source of persistence behavior: edits mark data blocks modified, and moving a viewer far enough to unload them saves those blocks into the selected stream.

## Increment 2: Region Files Stream

Add `voxel_core::streams::RegionFilesStream`, the missing forest wrapper over `RegionFile<StdVoxelFile>`. It implements `VoxelStream` and owns synchronized mutable state because individual region files are not thread-safe.

The stream maps a block position to:

- a region position using floor division by `1 << region_size_po2`;
- a wrapped local block position within that region;
- a separate `.vxr` file per `(region_position, lod)`.

It reads and writes upstream-compatible `meta.vxrm` metadata with format version 3, `block_size_po2`, `region_size_po2`, `sector_size`, and all channel depths. Defaults match upstream: block size power 4, region size power 4, and sector size 512. The first save creates the directory and metadata; later opens validate metadata before reading or writing blocks. Missing directories, metadata, regions, and blocks return `LoadResult::NotFound`; malformed metadata, incompatible formats, and I/O failures return `VoxelStreamError`.

An LRU cache keeps at most eight open region files. Eviction and `flush()` persist dirty headers before closing files. `Drop` performs a best-effort flush, while explicit `flush()` reports failures. All path construction uses structured `Path`/`PathBuf` operations, and metadata uses a structured JSON parser.

The Godot `VoxelStreamRegionFiles` Resource exposes `directory`, `block_size_po2`, `region_size_po2`, and `sector_size`. Configuration setters validate the same ranges as core metadata and apply before the stream is first used. It also exposes `flush() -> bool`; failures are logged through Godot and return `false`.

## Error Handling

Core code never panics for user-controlled paths or file contents. File and metadata failures are represented by `VoxelStreamError` with enough context to identify the operation and path. Godot-facing methods translate these errors to `godot_error!` and stable boolean/empty results rather than leaking Rust errors across the extension boundary.

Configuration that would invalidate already-open region files is rejected after first use. `VoxelTerrain` reports unsupported stream Resources once during initialization rather than once per paging tick.

## Testing

Tests follow red-green-refactor for each behavior:

- `MemoryStream` inspection tests cover block counting and clearing without changing round-trip behavior.
- Godot-layer unit tests cover Resource-to-trait resolution independently from scene rendering.
- Terrain/core integration tests prove an edited block is saved on unload and restored after a new terrain core is constructed with the same memory stream.
- Region-forest tests use temporary directories and cover positive and negative coordinates, LOD-separated files, cross-region round trips, metadata creation/reopen, incompatible metadata rejection, LRU eviction, and explicit flush.
- A compatibility test opens a deterministic upstream-layout fixture or verifies the exact `meta.vxrm` and `.vxr` naming/layout against the C++ contract.

Each increment must pass `cargo test -p voxel-core`, `cargo build -p voxel-gdext`, `cargo clippy --workspace --all-targets`, and `cargo fmt --all -- --check`. The verified baseline is 707 unit tests, 11 integration tests, and one doc-test before new coverage is added.

## Documentation Cleanup

After the code lands:

- Update `rust/STATUS.md` test totals and M3.18 progress.
- Append the completed stream-binding steps to the Phase 5 entry in `rust/AUDIT.md` without rewriting historical audit findings.
- Replace the obsolete Phase 2 hello-world status and examples in `rust/voxel-gdext/README.md` with the current terrain, generator, editing, material, collision, and stream API.
- Remove or correct stale source comments that describe `MemoryStream` as test-only once it becomes a supported runtime backend.

The two implementation increments and final documentation cleanup are separate commits. Unrelated working-tree content, including `.zcode/`, is left untouched.
