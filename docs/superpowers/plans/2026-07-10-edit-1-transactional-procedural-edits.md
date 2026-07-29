# EDIT-1: Transactional Procedural Edits Implementation Plan

> **Status:** completed and reconciled on 2026-07-24 (`d379d1b8`, `166511e0`, `cd4c229a`, integration `070a3fc6`).
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a LOD-0 single-voxel edit materialize an absent procedural block before mutation and mark it dirty atomically with that mutation.

**Architecture:** Add `SharedVoxelData::try_edit_voxel` as the high-level transactional entry point. It holds the block's `SpatialLock3D` write guard for the entire operation, snapshots settings and generator without a map lock, materializes a candidate buffer outside the map lock, then rechecks the map and mutates/marks the resident block under one map write lock. Existing low-level `try_set_voxel` and `mark_area_modified` stay available for existing multi-voxel paths.

**Tech Stack:** Rust 1.96.1, standard-library `RwLock`, `SpatialLock3D`, cargo test.

## Global Constraints

- Do not call `VoxelGenerator::generate_block` while holding a `VoxelDataMap` lock.
- Acquire the spatial write region before the LOD-0 map lock; do not expose a set-but-not-dirty state.
- Preserve current streaming semantics: a missing or empty block is not materialized when streaming is enabled or full load is incomplete.
- Do not modify the user's pending `rust/AUDIT.md` change in this work.

---

### Task 1: Lock in transactional edit regressions

**Files:**

- Modify: `rust/voxel-core/src/storage/voxel_data.rs:1541-2340`
- Test: `rust/voxel-core/src/storage/voxel_data.rs:tests`

**Interfaces:**

- Consumes: `SharedVoxelData::new(VoxelData)`, `SharedVoxelData::set_generator`, `SharedVoxelData::try_set_block`, `SharedVoxelData::unview_area`.
- Produces: failing calls to `SharedVoxelData::try_edit_voxel(value, pos, channel_index) -> bool`.

- [ ] **Step 1: Add a generator-backed missing-block regression test**

  In the existing `tests` module, add this test after `shared_voxel_data_region_locks_follow_voxel_data_contract`:

  ```rust
  #[test]
  fn shared_edit_voxel_materializes_procedural_block_and_marks_it_dirty() {
      let mut data = VoxelData::new();
      data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
      data.set_streaming_enabled(false);
      data.set_full_load_completed(true);
      data.set_generator(Some(Arc::new(RecordingGenerator::default())));
      let shared = SharedVoxelData::new(data);
      let channel = ChannelId::Type.index();

      assert!(shared.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel));

      let block = shared.block_snapshot(Vector3i::zero(), 0).unwrap();
      assert_eq!(block.voxels().get_voxel(1, 1, 1, channel), 99);
      assert_eq!(block.voxels().get_voxel(2, 1, 1, channel), 10);
      assert!(block.is_modified());
      assert!(block.is_edited());
  }
  ```

- [ ] **Step 2: Run the regression to verify it fails for the missing API**

  Run: `cargo test --manifest-path rust/Cargo.toml -p voxel-core shared_edit_voxel_materializes_procedural_block_and_marks_it_dirty`

  Expected: compilation fails because `SharedVoxelData::try_edit_voxel` does not exist.

- [ ] **Step 3: Add an immediate-unview save-candidate regression**

  Add a second test that calls `try_edit_voxel`, then holds `write_region(0, Box3i::new(Vector3i::zero(), Vector3i::splat(16)))` while invoking `unview_area`. Assert that `saves` contains exactly one `BlockToSave` at `(0, 0, 0)` with the edited value `77`; this proves the dirty flag is set before the edit method returns.

  ```rust
  #[test]
  fn shared_edit_voxel_is_dirty_before_immediate_unview() {
      let mut data = VoxelData::new();
      data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
      data.set_streaming_enabled(false);
      data.set_full_load_completed(true);
      let shared = SharedVoxelData::new(data);
      let channel = ChannelId::Type.index();

      assert!(shared.try_edit_voxel(77, Vector3i::new(1, 1, 1), channel));
      let mut saves = Vec::new();
      let area = Box3i::new(Vector3i::zero(), Vector3i::splat(1));
      let voxel_area = Box3i::new(Vector3i::zero(), Vector3i::splat(16));
      let _region = shared.write_region(0, voxel_area);
      shared.unview_area(area, 0, None, None, Some(&mut saves));

      assert_eq!(saves.len(), 1);
      assert_eq!(saves[0].position, Vector3i::zero());
      assert_eq!(saves[0].voxels.as_ref().unwrap().get_voxel(1, 1, 1, channel), 77);
  }
  ```

- [ ] **Step 4: Add the concurrent-insert recheck regression**

  Add a test-only `BlockingGenerator` with `entered` and `release` values of type `Arc<(Mutex<bool>, Condvar)>`. In `generate_block`, set `entered` to true and notify, then wait in a loop for `release` before filling the Type channel with `10`. Spawn `try_edit_voxel(99, Vector3i::new(1, 1, 1), channel)` on an `Arc<SharedVoxelData>`, wait until `entered` is true, insert a resident block with Type value `33` at `(2,1,1)`, notify `release`, then join the edit.

  ```rust
  struct BlockingGenerator {
      entered: Arc<(Mutex<bool>, Condvar)>,
      release: Arc<(Mutex<bool>, Condvar)>,
  }

  impl VoxelGenerator for BlockingGenerator {
      fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
          let (entered_lock, entered_cvar) = &*self.entered;
          *entered_lock.lock().unwrap() = true;
          entered_cvar.notify_one();
          let (release_lock, release_cvar) = &*self.release;
          let mut released = release_lock.lock().unwrap();
          while !*released {
              released = release_cvar.wait(released).unwrap();
          }
          input.buffer.fill(10, ChannelId::Type.index());
          GenResult::default()
      }
  }

  #[test]
  fn shared_edit_voxel_keeps_resident_block_inserted_during_materialization() {
      let mut data = VoxelData::new();
      data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
      data.set_streaming_enabled(false);
      data.set_full_load_completed(true);
      let entered = Arc::new((Mutex::new(false), Condvar::new()));
      let release = Arc::new((Mutex::new(false), Condvar::new()));
      data.set_generator(Some(Arc::new(BlockingGenerator {
          entered: entered.clone(),
          release: release.clone(),
      })));
      let shared = Arc::new(SharedVoxelData::new(data));
      let channel = ChannelId::Type.index();
      let edit_data = shared.clone();
      let edit = std::thread::spawn(move || {
          edit_data.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel)
      });

      let (entered_lock, entered_cvar) = &*entered;
      let mut started = entered_lock.lock().unwrap();
      while !*started {
          started = entered_cvar.wait(started).unwrap();
      }
      drop(started);
      let mut resident = VoxelBuffer::with_size(Vector3i::splat(16));
      resident.set_voxel(33, 2, 1, 1, channel);
      assert!(shared.try_set_block(Vector3i::zero(), VoxelDataBlock::with_voxels(resident, 0)));
      let (release_lock, release_cvar) = &*release;
      *release_lock.lock().unwrap() = true;
      release_cvar.notify_one();
      assert!(edit.join().unwrap());

      let block = shared.block_snapshot(Vector3i::zero(), 0).unwrap();
      assert_eq!(block.voxels().get_voxel(1, 1, 1, channel), 99);
      assert_eq!(block.voxels().get_voxel(2, 1, 1, channel), 33);
  }
  ```

- [ ] **Step 5: Add the generator map-lock regression**

  Define the following test-only generator and test. It must be added before implementation so the missing method produces the red failure; after implementation it proves generation happens outside the map write lock.

  ```rust
  struct MapUnlockProbeGenerator {
      data: std::sync::Weak<SharedVoxelData>,
  }

  impl VoxelGenerator for MapUnlockProbeGenerator {
      fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
          let data = self.data.upgrade().expect("shared data survives generation");
          drop(data.try_lod_map_write(0).expect("generator must run without map write lock"));
          input.buffer.fill(42, ChannelId::Type.index());
          GenResult::default()
      }
  }

  #[test]
  fn shared_edit_voxel_runs_generator_without_map_lock() {
      let mut data = VoxelData::new();
      data.set_bounds(Box3i::new(Vector3i::zero(), Vector3i::splat(16)));
      data.set_streaming_enabled(false);
      data.set_full_load_completed(true);
      let shared = Arc::new(SharedVoxelData::new(data));
      shared.set_generator(Some(Arc::new(MapUnlockProbeGenerator {
          data: Arc::downgrade(&shared),
      })));
      let channel = ChannelId::Type.index();

      assert!(shared.try_edit_voxel(99, Vector3i::new(1, 1, 1), channel));
      assert_eq!(shared.block_snapshot(Vector3i::zero(), 0).unwrap().voxels().get_voxel(2, 1, 1, channel), 42);
  }
  ```

- [ ] **Step 6: Run all edit regressions and verify they fail only because the API is absent**

  Run: `cargo test --manifest-path rust/Cargo.toml -p voxel-core shared_edit_voxel`

  Expected: compilation fails with no method named `try_edit_voxel`; no regression should fail for another reason.

- [ ] **Step 7: Commit the red tests**

  ```bash
  git add rust/voxel-core/src/storage/voxel_data.rs
  git commit -m "test(rust): cover transactional procedural edits"
  ```

### Task 2: Implement the LOD-0 transactional edit API

**Files:**

- Modify: `rust/voxel-core/src/storage/voxel_data.rs:13,371-421`
- Test: `rust/voxel-core/src/storage/voxel_data.rs:tests`

**Interfaces:**

- Consumes: `VoxelQueryData`, `SharedVoxelDataSettingsSnapshot`, `VoxelDataMap::voxel_to_block_b`, `VoxelDataMap::set_block_buffer`, `VoxelDataMap::set_voxel`.
- Produces: `pub fn SharedVoxelData::try_edit_voxel(&self, value: u64, pos: Vector3i, channel_index: usize) -> bool`.

- [ ] **Step 1: Import the generation query type**

  Replace the generator import with:

  ```rust
  use crate::generators::base::{VoxelGenerator, VoxelQueryData};
  ```

- [ ] **Step 2: Implement `try_edit_voxel` before the existing `try_set_voxel` method**

  Add this method. The private spatial guard is intentionally bound to `_write_region` so it remains alive through the map recheck and flag writes.

  ```rust
  pub fn try_edit_voxel(&self, value: u64, pos: Vector3i, channel_index: usize) -> bool {
      let settings = self.settings_snapshot();
      if !settings.bounds_in_voxels.contains_point(pos) {
          return false;
      }

      let block_size = self.block_size() as i32;
      let block_pos = VoxelDataMap::voxel_to_block_b(pos, self.block_size_po2());
      let block_box = Box3i::new(block_pos * block_size, Vector3i::splat(block_size));
      let _write_region = self.write_region(0, block_box);

      let needs_materialization = self.with_lod_map(0, |map| {
          map.get_block(block_pos)
              .is_none_or(|block| !block.has_voxels())
      });
      if needs_materialization && (settings.streaming_enabled || !settings.full_load_completed) {
          return false;
      }

      let mut prepared = needs_materialization.then(|| {
          let mut voxels = create_block_buffer(block_size, settings.format);
          if let Some(generator) = settings.generator {
              generator.generate_block(VoxelQueryData {
                  buffer: &mut voxels,
                  origin_in_voxels: block_pos * block_size,
                  lod: 0,
              });
          }
          voxels
      });

      self.with_lod_map_mut(0, |map| {
          let has_resident_voxels = map
              .get_block(block_pos)
              .is_some_and(|block| block.has_voxels());
          if !has_resident_voxels {
              map.set_block_buffer(
                  block_pos,
                  prepared.take().expect("materialization was prepared"),
                  true,
              );
          }

          map.set_voxel(value, pos, channel_index);
          let block = map
              .get_block_mut(block_pos)
              .expect("edited block exists after materialization");
          block.set_modified(true);
          block.set_edited(true);
      });
      true
  }
  ```

- [ ] **Step 3: Run the focused transactional edit tests**

  Run: `cargo test --manifest-path rust/Cargo.toml -p voxel-core shared_edit_voxel`

  Expected: PASS for all four tests added in Task 1.

- [ ] **Step 4: Run the storage module regression set**

  Run: `cargo test --manifest-path rust/Cargo.toml -p voxel-core storage::voxel_data::tests`

  Expected: PASS with no failures in existing `try_set_voxel`, `mark_area_modified`, view/unview, or shared-lock tests.

- [ ] **Step 5: Commit the implementation**

  ```bash
  git add rust/voxel-core/src/storage/voxel_data.rs
  git commit -m "fix(rust): make procedural voxel edits transactional"
  ```

### Task 3: Verify the completed edit path

**Files:**

- Test: `rust/voxel-core/src/storage/voxel_data.rs:tests`

**Interfaces:**

- Consumes: the four `shared_edit_voxel_*` regressions from Task 1 and `SharedVoxelData::try_edit_voxel` from Task 2.
- Produces: verification evidence for procedural preservation, immediate save eligibility, concurrent insertion preservation, and map-unlocked generation.

- [ ] **Step 1: Run all focused edit regressions**

  Run: `cargo test --manifest-path rust/Cargo.toml -p voxel-core shared_edit_voxel`

  Expected: PASS for all four transactional-edit regressions.

- [ ] **Step 2: Run formatting, clippy, and the full package suite**

  Run: `cargo fmt --manifest-path rust/Cargo.toml --all --check && cargo clippy --manifest-path rust/Cargo.toml -p voxel-core --all-targets --all-features -- -D warnings && cargo test --manifest-path rust/Cargo.toml -p voxel-core`

  Expected: all commands exit 0.

## Plan Review

- Spec coverage: Task 1 proves procedural preservation, immediate dirty state, map recheck, and unlocked generation; Task 2 implements materialization, atomic flags, and legacy streaming refusal; Task 3 runs all required gates.
- Type consistency: every task uses `SharedVoxelData::try_edit_voxel(value, pos, channel_index) -> bool`; only existing exported types are used.
- Scope: the plan does not change batch edits, map sharding, save retries, or the pending audit document.
