# VoxelEngine Task Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the threaded-task ownership and completed-task drain subset of Rust `VoxelEngine`.

**Architecture:** Extend `engine::voxel_engine` so `VoxelEngine` owns `ThreadedTaskRunner`. Add Rust-owned async/async-IO enqueue methods, explicit thread-count controls, wait/clear helpers, and make `process()` apply completed task results before syncing viewer priority data.

**Tech Stack:** Rust, `voxel-core`, existing `ThreadedTaskRunner`, `ThreadedTask`, and `PriorityViewersData`.

---

### Task 1: Threaded Task Ownership And Process Drain

**Files:**
- Modify: `rust/voxel-core/src/engine/voxel_engine.rs`
- Modify: `rust/STATUS.md`
- Modify: `MIGRATION_PLAN.md`

- [ ] **Step 1: Write failing tests**

Add tests in `rust/voxel-core/src/engine/voxel_engine.rs` for:
- `process_applies_completed_async_tasks_and_syncs_viewers`
- `async_io_tasks_run_serially_through_engine`
- `process_enqueues_and_applies_follow_up_tasks`
- `wait_and_clear_all_tasks_drops_completed_tasks_without_applying_results`

- [ ] **Step 2: Verify RED**

Run: `cargo test -p voxel-core engine::voxel_engine`

Expected: compile failure because `VoxelEngine::with_thread_count`, `push_async_task`, `push_async_io_task`, `wait_for_all_tasks`, and `wait_and_clear_all_tasks` are not implemented yet.

- [ ] **Step 3: Implement minimal code**

Add a `ThreadedTaskRunner` field to `VoxelEngine`, a `with_thread_count(thread_count: usize)` constructor, explicit thread count accessors, async/async-IO enqueue methods, `wait_for_all_tasks`, `wait_and_clear_all_tasks`, and update `process()` to drain/apply completed tasks before `sync_viewers_task_priority_data()`.

- [ ] **Step 4: Verify GREEN**

Run: `cargo test -p voxel-core engine::voxel_engine`

Expected: all `engine::voxel_engine` tests pass.

- [ ] **Step 5: Verify full slice**

Run:
- `cargo fmt --all -- --check`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets`
- `cargo build --workspace`
- `git diff --check`

- [ ] **Step 6: Commit and push**

Commit message: `rust(engine): add VoxelEngine task drain loop`
