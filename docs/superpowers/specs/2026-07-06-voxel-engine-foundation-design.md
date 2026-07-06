# VoxelEngine Foundation Design

**Goal:** Add the first engine-agnostic `VoxelEngine` layer needed by multi-volume terrain and future multi-LOD paging work.

**Scope:** This slice ports the C++ engine registry/viewer-priority subset only. It does not port task runners, GPU task queues, main-thread time-spread tasks, file locking, Godot callbacks, or the `VoxelLodTerrain` orchestrator.

**Architecture:** `voxel-core::engine::voxel_engine` owns generational slot maps for volumes and viewers. Viewers store world position, horizontal/vertical view distances, visual/collision/data-notification flags, and network peer id. `VoxelEngine::sync_viewers_task_priority_data` mirrors C++ by copying active viewer positions into shared `PriorityViewersData` and setting `highest_view_distance` to twice the maximum viewer distance.

**Testing:** Unit tests cover stale handles after removal/reuse, viewer property round-trips, shared priority sync, and `process()` calling the sync step.
