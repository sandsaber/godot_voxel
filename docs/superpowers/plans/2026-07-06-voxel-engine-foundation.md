# VoxelEngine Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the engine-agnostic registry/viewer-priority subset of `VoxelEngine`.

**Architecture:** Add `engine::voxel_engine` with generational ids, volume/viewer registries, viewer flags/distances, and shared `PriorityViewersData` sync. Keep task runners and Godot callbacks out of this slice.

**Tech Stack:** Rust, `voxel-core`, existing `Vector3f` and `PriorityViewersData`.

---

### Task 1: Registry And Viewer Priority Sync

**Files:**
- Create: `rust/voxel-core/src/engine/voxel_engine.rs`
- Modify: `rust/voxel-core/src/engine/mod.rs`
- Modify: `rust/STATUS.md`
- Modify: `MIGRATION_PLAN.md`

- [ ] **Step 1: Write failing tests**

Add tests for:
- stale `ViewerId` becomes invalid after remove and slot reuse
- viewer properties round-trip
- `sync_viewers_task_priority_data` writes active viewer positions and `max_distance * 2`
- `process()` invokes sync

- [ ] **Step 2: Verify RED**

Run: `cargo test -p voxel-core engine::voxel_engine`

Expected: compile failure because `VoxelEngine`, `ViewerDistances`, `VolumeId`, and `ViewerId` are not implemented yet.

- [ ] **Step 3: Implement minimal code**

Implement `VoxelEngine`, `Viewer`, `ViewerDistances`, `VolumeId`, `ViewerId`, and a private `GenerationalSlotMap<T>`.

- [ ] **Step 4: Verify GREEN**

Run: `cargo test -p voxel-core engine::voxel_engine`

Expected: all new tests pass.

- [ ] **Step 5: Verify full slice**

Run:
- `cargo fmt --all -- --check`
- `cargo test -p voxel-core`
- `cargo clippy --workspace --all-targets`
- `cargo build --workspace`

- [ ] **Step 6: Commit**

Commit message: `rust(engine): add VoxelEngine viewer registry foundation`
