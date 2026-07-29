# Wave 0D Save Journal and Shutdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent terrain save payload loss by retaining dirty block data until the matching save generation succeeds, retrying failed saves, preserving newest-write ordering, and exposing explicit shutdown flush.

**Architecture:** Add a terrain-level save journal keyed by `(position, lod)` because `VoxelData::unview_area` transfers ownership out of loaded data. Each queued save receives a monotonically increasing generation; task completion only clears the journal when it matches the latest generation and was not dropped. Failed or stale completions keep or requeue the latest payload.

**Tech Stack:** Rust, `voxel-core`, existing `ThreadedTaskRunner`, `SaveBlockDataTask`, in-memory streams.

## Global Constraints

- Do not change `VoxelData::unview_area` ownership behavior unless the journal cannot preserve payload safely.
- `SaveBlockDataTask` must return enough completion metadata to identify generation and failed/dropped state.
- Older save completions must never clear or overwrite newer pending save payloads.
- `shutdown_and_flush()` must wait for queued/in-flight saves, retry failures according to the same journal rules, flush the stream, and return `Err` if data remains unsaved.
- Do not implement multi-handle region-file coherency or crash-proof WAL in Wave 0D.
- Do not touch `rust/AUDIT.md`; it is user-owned working-tree state.

---

## File Structure

- Modify: `rust/voxel-core/src/streams/block_data_output.rs`
  - Add `save_generation: u64` to `BlockDataOutput`.
  - Make saved outputs optionally carry returned `VoxelBuffer` on dropped/failed save.
- Modify: `rust/voxel-core/src/streams/save_block_data_task.rs`
  - Accept generation, keep payload until save result is known, return payload on dropped/failed output.
- Modify: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`
  - Add `SaveKey`, `SaveJournalEntry`, `SaveFlushError`.
  - Queue saves serially.
  - Requeue failed latest saves.
  - Ignore stale completions.
  - Add `shutdown_and_flush`.
- Test: `save_block_data_task.rs` and `voxel_terrain_core.rs`.

### Task 1: Return Save Generation and Payload on Failure

**Files:**
- Modify: `rust/voxel-core/src/streams/block_data_output.rs`
- Modify: `rust/voxel-core/src/streams/save_block_data_task.rs`
- Test: `rust/voxel-core/src/streams/save_block_data_task.rs`

**Interfaces:**
- Consumes: existing `SaveBlockDataTask::new_voxels`.
- Produces: `SaveBlockDataTask::new_voxels_with_generation` and `BlockDataOutput.save_generation`.

- [ ] **Step 1: Add failing save-task tests**

Add to `save_block_data_task.rs` tests:

```rust
    #[test]
    fn failed_save_output_returns_generation_and_voxels() {
        let stream: Arc<dyn VoxelStream> = Arc::new(ErrorSaveStream);
        let dependency = StreamingDependency::new(stream);
        let mut task = SaveBlockDataTask::new_voxels_with_generation(
            Vector3i::new(1, 2, 3),
            0,
            Some(filled_buffer(55)),
            dependency,
            None,
            false,
            42,
        );

        task.run_save();
        let output = task.take_output().unwrap();

        assert_eq!(output.kind, BlockDataOutputKind::Saved);
        assert_eq!(output.save_generation, 42);
        assert!(output.dropped);
        assert_eq!(
            output.voxels.unwrap().get_voxel(1, 0, 0, ChannelId::Type.index()),
            55
        );
    }

    #[test]
    fn successful_save_output_keeps_generation_and_drops_local_payload() {
        let stream = Arc::new(MemoryStream::new());
        let dependency = StreamingDependency::new(stream);
        let mut task = SaveBlockDataTask::new_voxels_with_generation(
            Vector3i::new(1, 2, 3),
            0,
            Some(filled_buffer(55)),
            dependency,
            None,
            false,
            43,
        );

        task.run_save();
        let output = task.take_output().unwrap();

        assert_eq!(output.save_generation, 43);
        assert!(!output.dropped);
        assert!(output.voxels.is_none());
    }
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p voxel-core streams::save_block_data_task::tests::failed_save_output_returns_generation_and_voxels --locked`

Expected: FAIL because generation and returned payload do not exist.

- [ ] **Step 3: Extend BlockDataOutput**

Add field:

```rust
    pub save_generation: u64,
```

Set `save_generation: 0` in loaded, needs-generation, not-found, and loaded-dropped constructors.

Replace saved constructors with:

```rust
    pub fn saved(
        position_in_blocks: Vector3i,
        lod_index: u8,
        had_voxels: bool,
        save_generation: u64,
    ) -> Self {
        Self {
            kind: BlockDataOutputKind::Saved,
            voxels: None,
            position_in_blocks,
            lod_index,
            dropped: false,
            max_lod_hint: false,
            initial_load: false,
            had_voxels,
            save_generation,
        }
    }

    pub fn saved_dropped(
        position_in_blocks: Vector3i,
        lod_index: u8,
        voxels: Option<VoxelBuffer>,
        had_voxels: bool,
        save_generation: u64,
    ) -> Self {
        Self {
            kind: BlockDataOutputKind::Saved,
            voxels,
            position_in_blocks,
            lod_index,
            dropped: true,
            max_lod_hint: false,
            initial_load: false,
            had_voxels,
            save_generation,
        }
    }
```

Update existing tests to pass `0` where they call `BlockDataOutput::saved`.

- [ ] **Step 4: Extend SaveBlockDataTask**

Add field:

```rust
    save_generation: u64,
```

Keep existing constructor as wrapper:

```rust
    pub fn new_voxels(
        position_in_blocks: Vector3i,
        lod_index: u8,
        voxels: Option<VoxelBuffer>,
        stream_dependency: Arc<StreamingDependency>,
        tracker: Option<Arc<AsyncDependencyTracker>>,
        flush_on_last_tracked_task: bool,
    ) -> Self {
        Self::new_voxels_with_generation(
            position_in_blocks,
            lod_index,
            voxels,
            stream_dependency,
            tracker,
            flush_on_last_tracked_task,
            0,
        )
    }

    pub fn new_voxels_with_generation(
        position_in_blocks: Vector3i,
        lod_index: u8,
        voxels: Option<VoxelBuffer>,
        stream_dependency: Arc<StreamingDependency>,
        tracker: Option<Arc<AsyncDependencyTracker>>,
        flush_on_last_tracked_task: bool,
        save_generation: u64,
    ) -> Self {
        let had_voxels = voxels.is_some();
        Self {
            position_in_blocks,
            lod_index,
            voxels,
            stream_dependency,
            tracker,
            flush_on_last_tracked_task,
            has_run: false,
            had_voxels,
            stream_error: None,
            tracker_error: None,
            follow_up_tasks: Vec::new(),
            output: None,
            save_generation,
        }
    }
```

Change `run_save` so the payload is moved only after the stream result is known:

```rust
        let Some(voxels) = self.voxels.take() else {
            if let Some(tracker) = &self.tracker {
                tracker.abort();
            }
            self.output = Some(BlockDataOutput::saved_dropped(
                self.position_in_blocks,
                self.lod_index,
                None,
                self.had_voxels,
                self.save_generation,
            ));
            return;
        };

        let stream = self.stream_dependency.stream();
        let save_result = stream.save_voxel_block(VoxelSaveQuery::new(
            &voxels,
            self.position_in_blocks,
            self.lod_index,
        ));
        if let Err(error) = save_result {
            self.stream_error = Some(error);
        }
```

At output construction:

```rust
        self.output = Some(if self.stream_error.is_some() {
            BlockDataOutput::saved_dropped(
                self.position_in_blocks,
                self.lod_index,
                Some(voxels),
                self.had_voxels,
                self.save_generation,
            )
        } else {
            BlockDataOutput::saved(
                self.position_in_blocks,
                self.lod_index,
                self.had_voxels,
                self.save_generation,
            )
        });
```

- [ ] **Step 5: Run save-task tests**

Run: `cargo test -p voxel-core streams::save_block_data_task --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add rust/voxel-core/src/streams/block_data_output.rs rust/voxel-core/src/streams/save_block_data_task.rs
git commit -m "fix(rust): return failed save payloads"
```

### Task 2: Add Terrain Save Journal and Serial Save Queue

**Files:**
- Modify: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`
- Test: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`

**Interfaces:**
- Consumes: `BlockToSave`, `SaveBlockDataTask::new_voxels_with_generation`, save completion `BlockDataOutput`.
- Produces: terrain-level `save_journal`, generation tracking, serial save enqueue.

- [ ] **Step 1: Add failing terrain tests for failure retention**

Add test stream and test:

```rust
    struct FailThenMemoryStream {
        fails_remaining: AtomicUsize,
        inner: MemoryStream,
    }

    impl FailThenMemoryStream {
        fn new(fails: usize) -> Self {
            Self {
                fails_remaining: AtomicUsize::new(fails),
                inner: MemoryStream::new(),
            }
        }

        fn load_block(&self, position: Vector3i, lod: u8, out: &mut VoxelBuffer) -> LoadResult {
            self.inner.load_block(position, lod, out)
        }
    }

    impl VoxelStream for FailThenMemoryStream {
        fn save_voxel_block(
            &self,
            query: crate::streams::VoxelSaveQuery<'_>,
        ) -> crate::streams::StreamResult<()> {
            if self
                .fails_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
                .is_ok()
            {
                return Err(crate::streams::VoxelStreamError::Io("injected save failure".into()));
            }
            self.inner.save_voxel_block(crate::streams::VoxelSaveQuery::new(
                query.voxel_buffer,
                query.position_in_blocks,
                query.lod_index,
            ))
        }

        fn load_voxel_block(
            &self,
            query: crate::streams::VoxelLoadQuery<'_>,
        ) -> crate::streams::StreamResult<LoadResult> {
            self.inner.load_voxel_block(query)
        }

        fn flush(&self) -> crate::streams::StreamResult<()> {
            self.inner.flush()
        }
    }

    #[test]
    fn failed_unload_save_keeps_payload_and_retries() {
        let stream = Arc::new(FailThenMemoryStream::new(1));
        let mut core = build_core_with_stream(stream.clone());
        let bs = core.data_block_size();
        let channel = ChannelId::Type.index();
        let edited_voxel = Vector3i::new(1, 1, 1);
        let viewer = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];

        process_until(&mut core, &viewer, |core, _| {
            core.data().block_snapshot(Vector3i::zero(), 0).is_some()
        });
        assert!(core.data().try_set_voxel(88, edited_voxel, channel));
        core.data()
            .mark_area_modified(Box3i::new(edited_voxel, Vector3i::splat(1)), false);

        let empty_viewers = Vec::new();
        process_until(&mut core, &empty_viewers, |_core, _| {
            let mut loaded = VoxelBuffer::new(crate::storage::Allocator::Default);
            stream.load_block(Vector3i::zero(), 0, &mut loaded) == LoadResult::Found
                && loaded.get_voxel(1, 1, 1, channel) == 88
        });
    }
```

Add needed imports in the test module:

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p voxel-core terrain::voxel_terrain_core::tests::failed_unload_save_keeps_payload_and_retries --locked`

Expected: FAIL or timeout because failed saves are dropped and never retried.

- [ ] **Step 3: Add save journal types and fields**

Add near terrain structs:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SaveKey {
    position: Vector3i,
    lod_index: u8,
}

impl SaveKey {
    fn new(position: Vector3i, lod_index: u8) -> Self {
        Self { position, lod_index }
    }
}

#[derive(Debug)]
struct SaveJournalEntry {
    generation: u64,
    queued: bool,
    in_flight: bool,
    voxels: Option<VoxelBuffer>,
    retry_count: u32,
}
```

Add fields to `VoxelTerrainCore`:

```rust
    save_journal: HashMap<SaveKey, SaveJournalEntry>,
    next_save_generation: u64,
```

Initialize:

```rust
            save_journal: HashMap::new(),
            next_save_generation: 1,
```

- [ ] **Step 4: Journal enqueue and dispatch**

Replace `enqueue_data_save` with:

```rust
    fn enqueue_data_save(&mut self, save: BlockToSave) {
        let key = SaveKey::new(save.position, save.lod_index);
        let generation = self.next_save_generation;
        self.next_save_generation = self.next_save_generation.wrapping_add(1).max(1);
        let entry = self.save_journal.entry(key).or_insert_with(|| SaveJournalEntry {
            generation,
            queued: false,
            in_flight: false,
            voxels: None,
            retry_count: 0,
        });
        entry.generation = generation;
        entry.voxels = save.voxels;
        entry.queued = true;
        entry.retry_count = 0;
        self.dispatch_queued_save(key);
    }

    fn dispatch_queued_save(&mut self, key: SaveKey) {
        let Some(entry) = self.save_journal.get_mut(&key) else {
            return;
        };
        if !entry.queued || entry.in_flight {
            return;
        }
        let Some(voxels) = entry.voxels.take() else {
            return;
        };
        entry.queued = false;
        entry.in_flight = true;
        let task = SaveBlockDataTask::new_voxels_with_generation(
            key.position,
            key.lod_index,
            Some(voxels),
            StreamingDependency::new(self.stream.clone()),
            None,
            false,
            entry.generation,
        );
        self.task_runner.enqueue(Box::new(task), true);
    }
```

- [ ] **Step 5: Handle save completions separately**

In `drain_completed_tasks`, replace save branch:

```rust
            } else if let Some(output) = try_take_save_output(task.as_mut()) {
                self.apply_save_response(output);
```

Add method:

```rust
    fn apply_save_response(&mut self, output: BlockDataOutput) {
        let key = SaveKey::new(output.position_in_blocks, output.lod_index);
        let mut should_requeue = false;
        let mut remove_entry = false;

        if let Some(entry) = self.save_journal.get_mut(&key) {
            if output.save_generation != entry.generation {
                if output.dropped {
                    drop(output.voxels);
                }
                return;
            }
            entry.in_flight = false;
            if output.dropped {
                if entry.voxels.is_none() {
                    entry.voxels = output.voxels;
                }
                entry.queued = entry.voxels.is_some();
                entry.retry_count = entry.retry_count.saturating_add(1);
                should_requeue = entry.queued;
            } else {
                remove_entry = !entry.queued;
            }
        }

        if remove_entry {
            self.save_journal.remove(&key);
        } else if should_requeue {
            self.dispatch_queued_save(key);
        }
    }
```

- [ ] **Step 6: Run terrain journal test**

Run: `cargo test -p voxel-core terrain::voxel_terrain_core::tests::failed_unload_save_keeps_payload_and_retries --locked`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add rust/voxel-core/src/terrain/voxel_terrain_core.rs
git commit -m "fix(rust): retain failed terrain saves"
```

### Task 3: Preserve Newest Save Ordering for Same Block

**Files:**
- Modify: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`
- Test: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`

**Interfaces:**
- Consumes: save journal generation semantics.
- Produces: latest generation wins even if older completion arrives later.

- [ ] **Step 1: Add direct stale-completion test**

Add helper inside tests:

```rust
    fn terrain_with_empty_stream() -> VoxelTerrainCore {
        build_core_with_stream(Arc::new(MemoryStream::new()))
    }
```

Add test:

```rust
    #[test]
    fn stale_save_completion_does_not_clear_newer_journal_entry() {
        let mut core = terrain_with_empty_stream();
        let key = SaveKey::new(Vector3i::zero(), 0);
        core.save_journal.insert(
            key,
            SaveJournalEntry {
                generation: 2,
                queued: true,
                in_flight: true,
                voxels: Some(VoxelBuffer::with_size(Vector3i::splat(2))),
                retry_count: 0,
            },
        );

        core.apply_save_response(BlockDataOutput::saved(Vector3i::zero(), 0, true, 1));

        let entry = core.save_journal.get(&key).expect("newer save must remain");
        assert_eq!(entry.generation, 2);
        assert!(entry.queued);
        assert!(entry.in_flight);
        assert!(entry.voxels.is_some());
    }
```

- [ ] **Step 2: Run test**

Run: `cargo test -p voxel-core terrain::voxel_terrain_core::tests::stale_save_completion_does_not_clear_newer_journal_entry --locked`

Expected: PASS after Task 2; if it fails, fix `apply_save_response` generation comparison before continuing.

- [ ] **Step 3: Ensure queued newest save dispatches after in-flight older save completes**

Add this assertion test:

```rust
    #[test]
    fn latest_queued_save_dispatches_after_current_in_flight_save_finishes() {
        let mut core = terrain_with_empty_stream();
        let key = SaveKey::new(Vector3i::zero(), 0);
        core.save_journal.insert(
            key,
            SaveJournalEntry {
                generation: 1,
                queued: true,
                in_flight: true,
                voxels: Some(VoxelBuffer::with_size(Vector3i::splat(2))),
                retry_count: 0,
            },
        );

        core.apply_save_response(BlockDataOutput::saved(Vector3i::zero(), 0, true, 1));

        let entry = core.save_journal.get(&key).expect("queued latest save should be in flight");
        assert!(!entry.queued);
        assert!(entry.in_flight);
        assert!(entry.voxels.is_none());
    }
```

If this test fails because `remove_entry` runs before dispatch, change the success branch in `apply_save_response` to:

```rust
            } else if entry.queued {
                should_requeue = true;
            } else {
                remove_entry = true;
            }
```

- [ ] **Step 4: Run focused terrain save tests**

Run: `cargo test -p voxel-core terrain::voxel_terrain_core::tests::stale_save_completion_does_not_clear_newer_journal_entry terrain::voxel_terrain_core::tests::latest_queued_save_dispatches_after_current_in_flight_save_finishes --locked`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/voxel-core/src/terrain/voxel_terrain_core.rs
git commit -m "fix(rust): order terrain saves by generation"
```

### Task 4: Add Explicit Shutdown and Flush

**Files:**
- Modify: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`
- Test: `rust/voxel-core/src/terrain/voxel_terrain_core.rs`

**Interfaces:**
- Consumes: save journal and `ThreadedTaskRunner::shutdown`.
- Produces: `pub fn shutdown_and_flush(&mut self) -> Result<(), SaveFlushError>`.

- [ ] **Step 1: Add failing shutdown tests**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFlushError {
    Stream(crate::streams::VoxelStreamError),
    UnsavedBlocks { count: usize },
}
```

Then add tests:

```rust
    #[test]
    fn shutdown_and_flush_waits_for_pending_save() {
        let stream = Arc::new(MemoryStream::new());
        let mut core = build_core_with_stream(stream.clone());
        let bs = core.data_block_size();
        let channel = ChannelId::Type.index();
        let edited_voxel = Vector3i::new(1, 1, 1);
        let viewer = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];

        process_until(&mut core, &viewer, |core, _| {
            core.data().block_snapshot(Vector3i::zero(), 0).is_some()
        });
        assert!(core.data().try_set_voxel(99, edited_voxel, channel));
        core.data()
            .mark_area_modified(Box3i::new(edited_voxel, Vector3i::splat(1)), false);
        core.process(&[]);

        core.shutdown_and_flush().unwrap();

        let mut loaded = VoxelBuffer::new(crate::storage::Allocator::Default);
        assert_eq!(stream.load_block(Vector3i::zero(), 0, &mut loaded), LoadResult::Found);
        assert_eq!(loaded.get_voxel(1, 1, 1, channel), 99);
    }

    #[test]
    fn shutdown_and_flush_reports_unsaved_blocks_after_repeated_failures() {
        let stream = Arc::new(FailThenMemoryStream::new(usize::MAX));
        let mut core = build_core_with_stream(stream);
        let key = SaveKey::new(Vector3i::zero(), 0);
        core.save_journal.insert(
            key,
            SaveJournalEntry {
                generation: 1,
                queued: true,
                in_flight: false,
                voxels: Some(VoxelBuffer::with_size(Vector3i::splat(2))),
                retry_count: 0,
            },
        );
        core.dispatch_queued_save(key);

        assert!(matches!(
            core.shutdown_and_flush(),
            Err(SaveFlushError::UnsavedBlocks { count: 1 })
        ));
    }
```

- [ ] **Step 2: Run failing shutdown test**

Run: `cargo test -p voxel-core terrain::voxel_terrain_core::tests::shutdown_and_flush_waits_for_pending_save --locked`

Expected: FAIL because `shutdown_and_flush` does not exist.

- [ ] **Step 3: Implement shutdown and flush**

Add error type near public terrain structs:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFlushError {
    Stream(crate::streams::VoxelStreamError),
    UnsavedBlocks { count: usize },
}

impl std::fmt::Display for SaveFlushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stream(e) => write!(f, "terrain stream flush failed: {e}"),
            Self::UnsavedBlocks { count } => write!(f, "{count} terrain block saves remain unsaved"),
        }
    }
}

impl std::error::Error for SaveFlushError {}
```

Add to `VoxelTerrainCore`:

```rust
    pub fn shutdown_and_flush(&mut self) -> Result<(), SaveFlushError> {
        const MAX_SHUTDOWN_SAVE_ATTEMPTS: usize = 8;

        for _ in 0..MAX_SHUTDOWN_SAVE_ATTEMPTS {
            let keys: Vec<SaveKey> = self.save_journal.keys().copied().collect();
            for key in keys {
                self.dispatch_queued_save(key);
            }

            self.task_runner.wait_for_all_tasks();
            self.drain_completed_tasks();

            if self.save_journal.is_empty() {
                self.stream.flush().map_err(SaveFlushError::Stream)?;
                self.task_runner.shutdown();
                return Ok(());
            }
        }

        self.task_runner.wait_for_all_tasks();
        self.drain_completed_tasks();
        let count = self.save_journal.len();
        self.task_runner.shutdown();
        if count == 0 {
            self.stream.flush().map_err(SaveFlushError::Stream)
        } else {
            Err(SaveFlushError::UnsavedBlocks { count })
        }
    }
```

- [ ] **Step 4: Run shutdown tests**

Run: `cargo test -p voxel-core terrain::voxel_terrain_core::tests::shutdown_and_flush --locked`

Expected: PASS for both shutdown tests.

- [ ] **Step 5: Run all terrain core tests**

Run: `cargo test -p voxel-core terrain::voxel_terrain_core --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add rust/voxel-core/src/terrain/voxel_terrain_core.rs
git commit -m "feat(rust): flush terrain saves on shutdown"
```

### Task 5: Save Journal Verification

**Files:**
- Modify: none after previous tasks.
- Test: save task and terrain tests.

**Interfaces:**
- Consumes: Tasks 1-4.
- Produces: evidence that SAVE-1/SAVE-2 and shutdown flush behaviors are closed.

- [ ] **Step 1: Run focused save tests**

Run: `cargo test -p voxel-core streams::save_block_data_task terrain::voxel_terrain_core --locked`

Expected: PASS.

- [ ] **Step 2: Search for non-serial save enqueue**

Run: `rg -n "SaveBlockDataTask|enqueue\\(Box::new\\(task\\), false\\)" rust/voxel-core/src/terrain rust/voxel-core/src/streams`

Expected: terrain save enqueue uses `serial: true`; other false enqueue hits are load/mesh or unrelated tests.

- [ ] **Step 3: Commit if previous tasks were batched**

```bash
git add rust/voxel-core/src/streams/block_data_output.rs rust/voxel-core/src/streams/save_block_data_task.rs rust/voxel-core/src/terrain/voxel_terrain_core.rs
git commit -m "fix(rust): journal terrain save tasks"
```
