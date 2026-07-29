# VoxelEngine Task Loop Design

**Goal:** Add the engine-owned threaded task queue and completed-task drain loop to the Rust `VoxelEngine`.

**Scope:** This slice ports only the engine-agnostic threaded-task part of C++ `VoxelEngine::process`: async task enqueue, serial async-IO enqueue, waiting/clearing tasks, applying completed task results, follow-up task enqueueing, and viewer-priority sync. It does not port main-thread time-spread tasks, progressive tasks, GPU tasks, file locking, singleton lifecycle, stats/profiling, or volume callback dispatch.

**Architecture:** `voxel-core::engine::voxel_engine::VoxelEngine` owns one `ThreadedTaskRunner`. Public methods mirror the C++ intent with Rust ownership: `push_async_task`, `push_async_tasks`, `push_async_io_task`, `push_async_io_tasks`, `wait_for_all_tasks`, `wait_and_clear_all_tasks`, `thread_count`, and `set_thread_count`. `process()` drains completed threaded tasks via `drain_completed_tasks_and_enqueue_followups(false)`, calls `apply_result()` on each completed task, then refreshes shared viewer priority data.

**Testing:** Unit tests cover task results applying only on `process()`, serial IO task execution through the engine wrapper, follow-up tasks being enqueued and applied across process ticks, and `wait_and_clear_all_tasks()` dropping completed tasks without calling `apply_result()`.
