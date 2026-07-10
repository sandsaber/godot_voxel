//! Engine-agnostic single-LOD terrain paging orchestrator.
//!
//! Ports the engine-agnostic core of `terrain/fixed_lod/voxel_terrain.cpp`.
//! Drives [`VoxelData`] + [`MeshBlockTask`] from a set of paired viewers:
//! each [`process`](VoxelTerrainCore::process) tick diffs viewer positions
//! against the previous frame, loads/unloads data blocks via
//! [`VoxelData::view_area`] / [`VoxelData::unview_area`], requests mesh
//! updates for blocks whose neighbour data is resident, and runs the
//! pending load/mesh tasks through a [`ThreadedTaskRunner`].
//!
//! ## What is intentionally NOT here
//! - Godot `Node3D` / `RenderingServer` / `World3D` integration — that lives
//!   in the `voxel-gdext` crate (Phase 5).
//! - Instancer, multiplayer, collisions-as-separate-flag, quick-reload,
//!   save-on-unload, GPU generation, detail textures — all deferred.
//! - Multi-LOD paging (VoxelLodTerrain) — a separate orchestrator later.
//!
//! The minimum supported configuration is `mesh_block_size ==
//! data_block_size` (factor 1). The factor abstraction is preserved in the
//! helpers so a future patch can extend it without rewriting the hot path.

use crate::engine::{MeshingDependency, StreamingDependency};
use crate::math::{Box3i, Vector3i};
use crate::meshers::{
    BlockMeshOutput, MeshArraysPool, MeshBlockTask, MeshBlockTaskParams, MesherOutput,
};
use crate::storage::{BlockToSave, SharedVoxelData, VoxelBuffer, VoxelData, VoxelDataBlock};
use crate::streams::{
    BlockDataOutput, BlockDataOutputKind, MemoryStream, SaveBlockDataTask, VoxelStream,
    VoxelStreamError,
};
use crate::tasks::{ThreadedTask, ThreadedTaskRunner};
use std::collections::HashMap;
use std::sync::Arc;

/// Lightweight viewer identity (mirrors C++ `ViewerID`).
pub type ViewerId = u32;

/// Per-viewer cached state used to diff boxes between frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewerState {
    pub local_position_voxels: Vector3i,
    /// LOD-0 data box (backward compat for single-LOD code).
    pub data_box: Box3i,
    /// LOD-0 mesh box (backward compat for single-LOD code).
    pub mesh_box: Box3i,
    /// Per-LOD data boxes (index 0 = LOD 0). Empty when single-LOD.
    pub data_box_per_lod: Vec<Box3i>,
    /// Per-LOD mesh boxes. Empty when single-LOD.
    pub mesh_box_per_lod: Vec<Box3i>,
    pub horizontal_view_distance_voxels: i32,
    pub vertical_view_distance_voxels: i32,
    pub requires_meshes: bool,
}

/// A viewer the terrain is currently tracking.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PairedViewer {
    pub id: ViewerId,
    pub state: ViewerState,
    pub prev_state: ViewerState,
}

/// Rendered mesh block entry. Mirrors the per-block state of C++
/// `VoxelMeshBlockVT` minus the Godot mesh/collision resources. Carries the
/// most recent [`MesherOutput`] so a downstream renderer can upload it.
#[derive(Debug, Default)]
pub struct MeshBlockEntry {
    pub position: Vector3i,
    /// Number of viewers wanting a render mesh for this block.
    pub mesh_viewers: u32,
    /// `true` once at least one mesh result has been applied.
    pub is_loaded: bool,
    /// `true` while the block is queued in `blocks_pending_update`.
    pub is_in_update_list: bool,
    /// Most recent mesh output (set every time a mesh task completes).
    pub output: Option<MesherOutput>,
}

/// Optional notifier for terrain lifecycle events. Mirrors the C++ signals
/// `block_entered` / `block_exited` / `data_block_loaded` (the Rust core
/// surfaces them as a single sink so the Godot binding can route them).
#[derive(Debug, Default)]
pub struct VoxelTerrainStats {
    pub blocks_loaded: u64,
    pub blocks_unloaded: u64,
    pub meshes_built: u64,
    pub meshes_dropped: u64,
}

/// Lifecycle events emitted by [`VoxelTerrainCore::process`]. A Godot binding
/// can drain these to fire signals; tests inspect them to verify paging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoxelTerrainEvent {
    /// A data block finished loading (and was inserted into `VoxelData`).
    DataBlockLoaded(Vector3i),
    /// A data block was unloaded (viewers dropped to zero / out of range).
    DataBlockUnloaded(Vector3i),
    /// A mesh block produced geometry for the first time.
    MeshBlockEntered(Vector3i),
    /// A mesh block was unloaded (no more viewers).
    MeshBlockExited(Vector3i),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFlushError {
    Stream(VoxelStreamError),
    UnsavedBlocks { count: usize },
}

impl std::fmt::Display for SaveFlushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stream(e) => write!(f, "terrain stream flush failed: {e}"),
            Self::UnsavedBlocks { count } => {
                write!(f, "{count} terrain block saves remain unsaved")
            }
        }
    }
}

impl std::error::Error for SaveFlushError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SaveKey {
    position: Vector3i,
    lod_index: u8,
}

impl SaveKey {
    fn new(position: Vector3i, lod_index: u8) -> Self {
        Self {
            position,
            lod_index,
        }
    }
}

#[derive(Debug)]
struct SaveJournalEntry {
    generation: u64,
    queued: bool,
    in_flight_generation: Option<u64>,
    voxels: Option<VoxelBuffer>,
    retry_count: u32,
}

/// One paired viewer specification, used to add or update viewers.
#[derive(Debug, Clone, Copy)]
pub struct ViewerUpdate {
    pub id: ViewerId,
    pub world_position_voxels: Vector3i,
    pub horizontal_view_distance_voxels: i32,
    pub vertical_view_distance_voxels: i32,
    pub requires_meshes: bool,
}

/// Engine-agnostic paging terrain core (single- or multi-LOD).
pub struct VoxelTerrainCore {
    data: Arc<SharedVoxelData>,
    /// Number of LOD levels (1 = single-LOD backward compat).
    lod_count: u8,
    /// Per-LOD mesh block maps. Index 0 = LOD 0.
    mesh_maps: Vec<HashMap<Vector3i, MeshBlockEntry>>,
    paired_viewers: Vec<PairedViewer>,
    /// Per-LOD pending load positions.
    blocks_pending_load: Vec<Vec<Vector3i>>,
    /// Per-LOD pending mesh-update positions.
    blocks_pending_update: Vec<Vec<Vector3i>>,
    /// Per-LOD loading-block refcounts.
    loading_blocks: Vec<HashMap<Vector3i, u32>>,
    save_journal: HashMap<SaveKey, SaveJournalEntry>,
    next_save_generation: u64,
    meshing_dependency: Arc<MeshingDependency>,
    stream: Arc<dyn VoxelStream>,
    task_runner: ThreadedTaskRunner,
    mesh_arrays_pool: Arc<MeshArraysPool>,
    pub max_view_distance_voxels: i32,
    pub automatic_loading_enabled: bool,
    pub stats: VoxelTerrainStats,
    pub events: Vec<VoxelTerrainEvent>,
}

impl VoxelTerrainCore {
    /// Build a new terrain core over the given `VoxelData`. The terrain
    /// shares ownership of the data (so mesh tasks running on the
    /// [`ThreadedTaskRunner`] can lock it), and stores the mesher/stream in
    /// a [`MeshingDependency`] so swapping either invalidates in-flight work.
    pub fn new(
        data: VoxelData,
        stream: Arc<dyn VoxelStream>,
        meshing_dependency: Arc<MeshingDependency>,
    ) -> Self {
        Self::new_with_lod_count(data, stream, meshing_dependency, 1)
    }

    /// Build a multi-LOD terrain core. `lod_count` must be ≥ 1. The underlying
    /// `VoxelData` is configured with the matching LOD cascade.
    pub fn new_with_lod_count(
        mut data: VoxelData,
        stream: Arc<dyn VoxelStream>,
        meshing_dependency: Arc<MeshingDependency>,
        lod_count: u8,
    ) -> Self {
        assert!(lod_count >= 1, "lod_count must be >= 1");
        data.set_lod_count(lod_count as usize);
        let data = Arc::new(SharedVoxelData::new(data));
        let task_runner = ThreadedTaskRunner::new(num_threads());
        let n = lod_count as usize;
        Self {
            data,
            lod_count,
            mesh_maps: (0..n).map(|_| HashMap::new()).collect(),
            paired_viewers: Vec::new(),
            blocks_pending_load: (0..n).map(|_| Vec::new()).collect(),
            blocks_pending_update: (0..n).map(|_| Vec::new()).collect(),
            loading_blocks: (0..n).map(|_| HashMap::new()).collect(),
            save_journal: HashMap::new(),
            next_save_generation: 1,
            meshing_dependency,
            stream,
            task_runner,
            mesh_arrays_pool: Arc::new(MeshArraysPool::new()),
            max_view_distance_voxels: 192,
            automatic_loading_enabled: true,
            stats: VoxelTerrainStats::default(),
            events: Vec::new(),
        }
    }

    /// Convenience constructor for generator-only setups (no stream). A
    /// `MemoryStream` is used as a no-op sink; all loads fall back to the
    /// generator installed on `VoxelData`.
    pub fn new_generator_only(data: VoxelData, meshing_dependency: Arc<MeshingDependency>) -> Self {
        let stream: Arc<dyn VoxelStream> = Arc::new(MemoryStream::new());
        Self::new(data, stream, meshing_dependency)
    }

    pub fn data(&self) -> Arc<SharedVoxelData> {
        self.data.clone()
    }

    /// Returns the data block size (in voxels) used by the underlying
    /// `VoxelData`. The current port assumes mesh block size == data block
    /// size (factor 1).
    fn data_block_size(&self) -> i32 {
        // Matches the C++ inline `get_data_block_size()`.
        self.data.block_size() as i32
    }

    /// Reference to the LOD-0 mesh-block hashmap (read-only). Tests and
    /// the future Godot binding use this to find blocks needing upload.
    pub fn mesh_blocks(&self) -> &HashMap<Vector3i, MeshBlockEntry> {
        &self.mesh_maps[0]
    }

    /// Reference to the mesh-block hashmap for a specific LOD.
    pub fn mesh_blocks_at_lod(&self, lod: u8) -> &HashMap<Vector3i, MeshBlockEntry> {
        &self.mesh_maps[lod as usize]
    }

    /// Number of LOD levels.
    pub fn lod_count(&self) -> u8 {
        self.lod_count
    }

    /// Per-frame entry point: pump viewer updates, enqueue pending work, and
    /// drain any task outputs that have completed so far. Returns the events
    /// emitted this tick.
    ///
    /// Pass the desired viewer set via `viewers`; any paired viewer not in
    /// the list is treated as removed (its boxes shrink to empty, triggering
    /// unloads). This mirrors the C++ `process_viewers` + `process_meshing`
    /// pair, plus the `apply_*_response` callbacks folded in.
    pub fn process(&mut self, viewers: &[ViewerUpdate]) -> Vec<VoxelTerrainEvent> {
        self.events.clear();
        if !self.automatic_loading_enabled {
            return std::mem::take(&mut self.events);
        }

        self.drain_completed_tasks();
        self.process_viewers(viewers);
        self.send_data_load_requests();
        self.drain_completed_tasks();

        self.process_meshing();
        self.drain_completed_tasks();

        std::mem::take(&mut self.events)
    }

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
                let flush_result = self.stream.flush().map_err(SaveFlushError::Stream);
                self.task_runner.shutdown();
                return flush_result;
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

    fn process_viewers(&mut self, viewers: &[ViewerUpdate]) {
        let data_block_size = self.data_block_size();
        // Mesh block size == data block size for now (factor 1).
        let mesh_block_size = data_block_size;

        // Update paired viewers, recording prev_state for diffing.
        let mut seen = Vec::with_capacity(viewers.len());
        for update in viewers {
            seen.push(update.id);
            let paired = self.paired_viewers.iter_mut().find(|p| p.id == update.id);
            let horizontal = update
                .horizontal_view_distance_voxels
                .min(self.max_view_distance_voxels);
            let vertical = update
                .vertical_view_distance_voxels
                .min(self.max_view_distance_voxels);
            if let Some(paired) = paired {
                paired.prev_state = paired.state.clone();
                paired.state.local_position_voxels = update.world_position_voxels;
                paired.state.horizontal_view_distance_voxels = horizontal;
                paired.state.vertical_view_distance_voxels = vertical;
                paired.state.requires_meshes = update.requires_meshes;
            } else {
                let state = ViewerState {
                    local_position_voxels: update.world_position_voxels,
                    horizontal_view_distance_voxels: horizontal,
                    vertical_view_distance_voxels: vertical,
                    requires_meshes: update.requires_meshes,
                    ..ViewerState::default()
                };
                self.paired_viewers.push(PairedViewer {
                    id: update.id,
                    state: state.clone(),
                    prev_state: ViewerState::default(),
                });
            }
        }

        // Compute boxes for every paired viewer (new and updated alike).
        for paired in self.paired_viewers.iter_mut() {
            if !seen.contains(&paired.id) {
                paired.prev_state = paired.state.clone();
                paired.state.data_box = Box3i::default();
                paired.state.mesh_box = Box3i::default();
                paired.state.data_box_per_lod = Vec::new();
                paired.state.mesh_box_per_lod = Vec::new();
                paired.state.requires_meshes = false;
                continue;
            }
            if self.lod_count > 1 {
                compute_viewer_boxes_multi_lod(&mut paired.state, data_block_size, self.lod_count);
            } else {
                compute_viewer_boxes(&mut paired.state, data_block_size, mesh_block_size);
            }
        }

        // Diff each viewer and apply view/unview operations.
        if self.lod_count > 1 {
            self.process_viewers_multi_lod();
        } else {
            self.process_viewers_single_lod();
        }

        // Drop unpaired viewers from the list now that their boxes have
        // collapsed (matches the C++ swap-and-pop at the end of
        // process_viewers).
        self.paired_viewers.retain(|p| seen.contains(&p.id));
    }

    /// Single-LOD diff path (backward compat: lod_count == 1).
    fn process_viewers_single_lod(&mut self) {
        let mut data_unview_boxes: Vec<Box3i> = Vec::new();
        let mut data_view_boxes: Vec<Box3i> = Vec::new();
        let mut mesh_unview_positions: Vec<Vector3i> = Vec::new();
        let mut mesh_view_positions: Vec<Vector3i> = Vec::new();

        for paired in self.paired_viewers.iter() {
            if paired.prev_state.data_box != paired.state.data_box {
                for box_removed in paired.prev_state.data_box.difference(paired.state.data_box) {
                    data_unview_boxes.push(box_removed);
                }
                for box_added in paired.state.data_box.difference(paired.prev_state.data_box) {
                    data_view_boxes.push(box_added);
                }
            }
            if paired.prev_state.mesh_box != paired.state.mesh_box {
                for slab in paired.prev_state.mesh_box.difference(paired.state.mesh_box) {
                    for pos in slab.iter_cells_zxy() {
                        mesh_unview_positions.push(pos);
                    }
                }
                for slab in paired.state.mesh_box.difference(paired.prev_state.mesh_box) {
                    for pos in slab.iter_cells_zxy() {
                        mesh_view_positions.push(pos);
                    }
                }
            }
        }

        for box_unviewed in data_unview_boxes {
            self.apply_data_unview(box_unviewed, 0);
        }
        for box_viewed in data_view_boxes {
            self.apply_data_view(box_viewed, 0);
        }
        for pos in &mesh_unview_positions {
            self.unview_mesh_block(*pos, 0);
        }
        for pos in &mesh_view_positions {
            self.view_mesh_block(*pos, 0);
        }
    }

    /// Multi-LOD diff path: diff per-LOD boxes and dispatch view/unview per LOD.
    fn process_viewers_multi_lod(&mut self) {
        let lod_count = self.lod_count as usize;
        // Collect ops per LOD (to avoid borrow conflicts).
        let mut data_unview: Vec<Vec<Box3i>> = vec![Vec::new(); lod_count];
        let mut data_view: Vec<Vec<Box3i>> = vec![Vec::new(); lod_count];
        let mut mesh_unview: Vec<Vec<Vector3i>> = vec![Vec::new(); lod_count];
        let mut mesh_view: Vec<Vec<Vector3i>> = vec![Vec::new(); lod_count];

        for paired in self.paired_viewers.iter() {
            for lod in 0..lod_count {
                let prev_data = paired
                    .prev_state
                    .data_box_per_lod
                    .get(lod)
                    .copied()
                    .unwrap_or_default();
                let curr_data = paired
                    .state
                    .data_box_per_lod
                    .get(lod)
                    .copied()
                    .unwrap_or_default();
                if prev_data != curr_data {
                    for b in prev_data.difference(curr_data) {
                        data_unview[lod].push(b);
                    }
                    for b in curr_data.difference(prev_data) {
                        data_view[lod].push(b);
                    }
                }
                let prev_mesh = paired
                    .prev_state
                    .mesh_box_per_lod
                    .get(lod)
                    .copied()
                    .unwrap_or_default();
                let curr_mesh = paired
                    .state
                    .mesh_box_per_lod
                    .get(lod)
                    .copied()
                    .unwrap_or_default();
                if prev_mesh != curr_mesh {
                    for slab in prev_mesh.difference(curr_mesh) {
                        for pos in slab.iter_cells_zxy() {
                            mesh_unview[lod].push(pos);
                        }
                    }
                    for slab in curr_mesh.difference(prev_mesh) {
                        for pos in slab.iter_cells_zxy() {
                            mesh_view[lod].push(pos);
                        }
                    }
                }
            }
        }

        for lod in 0..lod_count {
            for box_ in &data_unview[lod] {
                self.apply_data_unview(*box_, lod);
            }
            for box_ in &data_view[lod] {
                self.apply_data_view(*box_, lod);
            }
            for pos in &mesh_unview[lod] {
                self.unview_mesh_block(*pos, lod);
            }
            for pos in &mesh_view[lod] {
                self.view_mesh_block(*pos, lod);
            }
        }
    }

    fn apply_data_view(&mut self, box_to_load: Box3i, lod: usize) {
        let mut missing = Vec::new();
        let mut found_positions = Vec::new();
        {
            let voxel_box = block_box_to_voxel_box(box_to_load, self.data_block_size());
            let _write_region = self.data.write_region(lod, voxel_box);
            self.data.view_area(
                box_to_load,
                lod,
                Some(&mut missing),
                Some(&mut found_positions),
                None,
            );
        }
        for bpos in missing {
            let entry = self.loading_blocks[lod].entry(bpos).or_insert(0);
            *entry += 1;
            if *entry == 1 {
                self.blocks_pending_load[lod].push(bpos);
            }
        }
        let _ = found_positions;
    }

    fn apply_data_unview(&mut self, box_to_unload: Box3i, lod: usize) {
        let mut removed_positions = Vec::new();
        let mut missing_positions = Vec::new();
        let mut saves = Vec::new();
        {
            let voxel_box = block_box_to_voxel_box(box_to_unload, self.data_block_size());
            let _write_region = self.data.write_region(lod, voxel_box);
            self.data.unview_area(
                box_to_unload,
                lod,
                Some(&mut removed_positions),
                Some(&mut missing_positions),
                Some(&mut saves),
            );
        }
        for save in saves {
            self.enqueue_data_save(save);
        }
        for bpos in removed_positions {
            self.stats.blocks_unloaded += 1;
            self.events.push(VoxelTerrainEvent::DataBlockUnloaded(bpos));
            self.loading_blocks[lod].remove(&bpos);
            self.blocks_pending_load[lod].retain(|p| *p != bpos);
        }
        for bpos in missing_positions {
            if let Some(count) = self.loading_blocks[lod].get_mut(&bpos) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.loading_blocks[lod].remove(&bpos);
                    self.blocks_pending_load[lod].retain(|p| *p != bpos);
                }
            }
        }
    }

    fn enqueue_data_save(&mut self, save: BlockToSave) {
        let key = SaveKey::new(save.position, save.lod_index);
        let generation = self.next_save_generation;
        self.next_save_generation = self.next_save_generation.wrapping_add(1).max(1);
        let entry = self
            .save_journal
            .entry(key)
            .or_insert_with(|| SaveJournalEntry {
                generation,
                queued: false,
                in_flight_generation: None,
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
        if !entry.queued || entry.in_flight_generation.is_some() {
            return;
        }
        let Some(voxels) = entry.voxels.take() else {
            return;
        };
        entry.queued = false;
        entry.in_flight_generation = Some(entry.generation);
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

    fn view_mesh_block(&mut self, bpos: Vector3i, lod: usize) {
        let entry = self.mesh_maps[lod]
            .entry(bpos)
            .or_insert_with(|| MeshBlockEntry {
                position: bpos,
                ..MeshBlockEntry::default()
            });
        entry.mesh_viewers += 1;
        self.try_schedule_mesh_update(bpos, lod);
    }

    fn unview_mesh_block(&mut self, bpos: Vector3i, lod: usize) {
        let Some(entry) = self.mesh_maps[lod].get_mut(&bpos) else {
            return;
        };
        if entry.mesh_viewers > 0 {
            entry.mesh_viewers -= 1;
        }
        if entry.mesh_viewers == 0 {
            let was_loaded = entry.is_loaded;
            let pool = self.mesh_arrays_pool.clone();
            if let Some(removed) = self.mesh_maps[lod].remove(&bpos) {
                if let Some(prev) = removed.output {
                    release_mesh_arrays_owned(&pool, prev);
                }
            }
            self.blocks_pending_update[lod].retain(|p| *p != bpos);
            if was_loaded {
                self.events.push(VoxelTerrainEvent::MeshBlockExited(bpos));
            }
        }
    }

    fn try_schedule_mesh_update(&mut self, bpos: Vector3i, lod: usize) {
        let already = self.mesh_maps[lod]
            .get(&bpos)
            .map(|e| e.is_in_update_list)
            .unwrap_or(false);
        if already {
            return;
        }
        let Some(entry) = self.mesh_maps[lod].get(&bpos) else {
            return;
        };
        if entry.mesh_viewers == 0 {
            return;
        }
        let data_box = Box3i::new(bpos, Vector3i::splat(1)).padded(1);
        let voxel_box = block_box_to_voxel_box(data_box, self.data_block_size());
        let _read_region = self.data.read_region(lod, voxel_box);
        if !self.data.has_all_blocks_in_area(data_box, lod) {
            return;
        }
        let entry = self.mesh_maps[lod]
            .get_mut(&bpos)
            .expect("mesh block exists after the early return");
        entry.is_in_update_list = true;
        self.blocks_pending_update[lod].push(bpos);
    }

    fn try_schedule_mesh_update_from_data(&mut self, voxel_box: Box3i, lod: usize) {
        let padded = voxel_box.padded(1);
        let data_block_size = self.data_block_size();
        let mesh_box = padded.downscaled(data_block_size);
        for bpos in mesh_box.iter_cells_zxy() {
            if self.mesh_maps[lod].contains_key(&bpos) {
                self.try_schedule_mesh_update(bpos, lod);
            }
        }
    }

    fn send_data_load_requests(&mut self) {
        let data = self.data.clone();
        let stream = self.stream.clone();
        let mut all_tasks: Vec<Box<dyn ThreadedTask>> = Vec::new();
        for lod in 0..self.lod_count as usize {
            if self.blocks_pending_load[lod].is_empty() {
                continue;
            }
            let positions = std::mem::take(&mut self.blocks_pending_load[lod]);
            let tasks = positions.into_iter().map(|bpos| {
                Box::new(LoadBlockForTerrainTask::new(
                    bpos,
                    lod as u8,
                    data.clone(),
                    stream.clone(),
                )) as Box<dyn ThreadedTask>
            });
            all_tasks.extend(tasks);
        }
        if !all_tasks.is_empty() {
            self.task_runner.enqueue_many(all_tasks, false);
        }
    }

    fn drain_completed_tasks(&mut self) {
        let completed = self
            .task_runner
            .drain_completed_tasks_and_enqueue_followups(false);
        for mut task in completed {
            if let Some(output) = try_take_load_output(task.as_mut()) {
                self.apply_data_block_response(output);
            } else if let Some(output) = try_take_save_output(task.as_mut()) {
                self.apply_save_response(output);
            } else if let Some(output) = try_take_mesh_output(task) {
                self.apply_mesh_update(output);
            }
        }
    }

    fn process_meshing(&mut self) {
        let data = self.data.clone();
        let meshing_dependency = self.meshing_dependency.clone();
        let mesh_arrays_pool = self.mesh_arrays_pool.clone();
        let mut all_tasks: Vec<Box<dyn ThreadedTask>> = Vec::new();
        for lod in 0..self.lod_count as usize {
            if self.blocks_pending_update[lod].is_empty() {
                continue;
            }
            let positions = std::mem::take(&mut self.blocks_pending_update[lod]);
            let tasks = positions.into_iter().map(|bpos| {
                if let Some(entry) = self.mesh_maps[lod].get_mut(&bpos) {
                    entry.is_in_update_list = false;
                }
                Box::new(MeshBlockTask::new(MeshBlockTaskParams {
                    position_in_blocks: bpos,
                    lod_index: lod as u8,
                    data: data.clone(),
                    meshing_dependency: meshing_dependency.clone(),
                    collision_hint: false,
                    lod_hint: false,
                    mesh_arrays_pool: Some(mesh_arrays_pool.clone()),
                })) as Box<dyn ThreadedTask>
            });
            all_tasks.extend(tasks);
        }
        if !all_tasks.is_empty() {
            self.task_runner.enqueue_many(all_tasks, false);
        }
    }

    /// Apply a data-load result (the C++ `apply_data_block_response`).
    pub fn apply_data_block_response(&mut self, output: BlockDataOutput) {
        let bpos = output.position_in_blocks;
        let lod = output.lod_index as usize;
        match output.kind {
            BlockDataOutputKind::Loaded | BlockDataOutputKind::NeedsGeneration => {
                if output.dropped {
                    if self.loading_blocks[lod].contains_key(&bpos) {
                        self.blocks_pending_load[lod].push(bpos);
                    }
                    return;
                }
                let Some(voxels) = output.voxels else {
                    return;
                };
                let viewer_count = self.loading_blocks[lod].remove(&bpos).unwrap_or(0);
                if viewer_count == 0 {
                    return;
                }
                let mut block = VoxelDataBlock::with_voxels(voxels, lod as u8);
                for _ in 0..viewer_count {
                    block.viewers.add();
                }
                block.set_edited(true);
                let inserted = {
                    let bs = self.data_block_size();
                    let lod_stride = 1i32 << lod;
                    let voxel_box = Box3i::new(bpos * bs * lod_stride, Vector3i::splat(bs));
                    let _write_region = self.data.write_region(lod, voxel_box);
                    self.data.try_set_block(bpos, block)
                };
                if inserted {
                    self.stats.blocks_loaded += 1;
                    self.events.push(VoxelTerrainEvent::DataBlockLoaded(bpos));
                    let bs = self.data_block_size();
                    let lod_stride = 1i32 << lod;
                    let voxel_box = Box3i::new(bpos * bs * lod_stride, Vector3i::splat(bs));
                    self.try_schedule_mesh_update_from_data(voxel_box, lod);
                }
            }
            BlockDataOutputKind::NotFound => {
                self.loading_blocks[lod].remove(&bpos);
            }
            BlockDataOutputKind::Saved => {}
        }
    }

    fn apply_save_response(&mut self, output: BlockDataOutput) {
        let key = SaveKey::new(output.position_in_blocks, output.lod_index);
        let mut should_dispatch = false;
        let mut should_remove = false;

        if let Some(entry) = self.save_journal.get_mut(&key) {
            if entry.in_flight_generation != Some(output.save_generation) {
                return;
            }
            entry.in_flight_generation = None;

            if output.save_generation != entry.generation {
                should_dispatch = entry.queued;
            } else if output.dropped {
                if entry.voxels.is_none() {
                    entry.voxels = output.voxels;
                }
                entry.queued = entry.voxels.is_some();
                entry.retry_count = entry.retry_count.saturating_add(1);
                should_dispatch = entry.queued;
            } else if entry.queued {
                should_dispatch = true;
            } else {
                should_remove = true;
            }
        }

        if should_remove {
            self.save_journal.remove(&key);
        } else if should_dispatch {
            self.dispatch_queued_save(key);
        }
    }

    /// Apply a mesh result (the C++ `apply_mesh_update`).
    pub fn apply_mesh_update(&mut self, output: BlockMeshOutput) {
        let bpos = output.position_in_blocks;
        let lod = output.lod_index as usize;
        let pool = self.mesh_arrays_pool.clone();
        if output.dropped {
            self.stats.meshes_dropped += 1;
            return;
        }
        let Some(entry) = self.mesh_maps[lod].get_mut(&bpos) else {
            release_mesh_arrays_owned(&pool, output.surfaces);
            self.stats.meshes_dropped += 1;
            return;
        };
        let became_loaded = !entry.is_loaded && !output.surfaces.is_empty();
        if let Some(prev) = entry.output.as_mut() {
            if let Some(arrays) = prev.take_first_transvoxel_arrays() {
                pool.release(arrays);
            }
        }
        entry.output = Some(output.surfaces);
        entry.is_loaded = true;
        self.stats.meshes_built += 1;
        if became_loaded {
            self.events.push(VoxelTerrainEvent::MeshBlockEntered(bpos));
        }
    }
}

/// Free helper: return any transvoxel `MeshArrays` from an owned `MesherOutput`
/// to the pool. Used at unload/dropped paths where the output is consumed
/// wholesale (audit §9.6-B3). Lives outside the `impl` so it can be called with
/// a cloned pool `Arc` while a mesh-map entry is mutably borrowed.
fn release_mesh_arrays_owned(pool: &MeshArraysPool, mut output: MesherOutput) {
    if let Some(arrays) = output.take_first_transvoxel_arrays() {
        pool.release(arrays);
    }
}

trait BoxDiff {
    fn difference(self, other: Box3i) -> Vec<Box3i>;
}

impl BoxDiff for Box3i {
    fn difference(self, other: Box3i) -> Vec<Box3i> {
        // C++ Box3i::difference_to_vec produces up to 6 slabs. We need the
        // same here to enumerate cells in the removed region efficiently.
        // If `other` doesn't intersect `self`, the entire `self` is removed.
        if self.size.x <= 0 || self.size.y <= 0 || self.size.z <= 0 {
            return Vec::new();
        }
        if !self.intersects(&other) {
            return vec![self];
        }
        let clip = self.clipped(other);
        if clip.size == self.size {
            // `other` fully covers `self`: nothing remains.
            return Vec::new();
        }
        // Compute the up-to-6 surrounding slabs by subtracting the clipped
        // region from each face in turn. Order: -X, +X, -Y, +Y, -Z, +Z.
        let mut slabs = Vec::new();
        let self_min = self.position;
        let self_max = self.position + self.size;
        let clip_min = clip.position;
        let clip_max = clip.position + clip.size;

        // -X slab
        if clip_min.x > self_min.x {
            slabs.push(Box3i::new(
                Vector3i::new(self_min.x, self_min.y, self_min.z),
                Vector3i::new(clip_min.x - self_min.x, self.size.y, self.size.z),
            ));
        }
        // +X slab
        if self_max.x > clip_max.x {
            slabs.push(Box3i::new(
                Vector3i::new(clip_max.x, self_min.y, self_min.z),
                Vector3i::new(self_max.x - clip_max.x, self.size.y, self.size.z),
            ));
        }
        // -Y slab (X already clipped)
        if clip_min.y > self_min.y {
            slabs.push(Box3i::new(
                Vector3i::new(clip_min.x, self_min.y, self_min.z),
                Vector3i::new(clip.size.x, clip_min.y - self_min.y, self.size.z),
            ));
        }
        // +Y slab
        if self_max.y > clip_max.y {
            slabs.push(Box3i::new(
                Vector3i::new(clip_min.x, clip_max.y, self_min.z),
                Vector3i::new(clip.size.x, self_max.y - clip_max.y, self.size.z),
            ));
        }
        // -Z slab (X and Y already clipped)
        if clip_min.z > self_min.z {
            slabs.push(Box3i::new(
                Vector3i::new(clip_min.x, clip_min.y, self_min.z),
                Vector3i::new(clip.size.x, clip.size.y, clip_min.z - self_min.z),
            ));
        }
        // +Z slab
        if self_max.z > clip_max.z {
            slabs.push(Box3i::new(
                Vector3i::new(clip_min.x, clip_min.y, clip_max.z),
                Vector3i::new(clip.size.x, clip.size.y, self_max.z - clip_max.z),
            ));
        }
        slabs
    }
}

/// Compute the data and mesh boxes for one viewer. Equivalent to C++
/// `process_viewers` Step E.
fn compute_viewer_boxes(state: &mut ViewerState, data_block_size: i32, mesh_block_size: i32) {
    let _ = mesh_block_size; // factor == 1 for now
    if !state.requires_meshes {
        // No mesh wanted: just keep data resident around the viewer.
        let h_blocks = ceil_div(state.horizontal_view_distance_voxels, data_block_size);
        let v_blocks = ceil_div(state.vertical_view_distance_voxels, data_block_size);
        let block_pos = floor_div_vec(state.local_position_voxels, data_block_size);
        state.mesh_box = Box3i::default();
        state.data_box =
            Box3i::from_center_extents(block_pos, Vector3i::new(h_blocks, v_blocks, h_blocks));
        return;
    }

    let mesh_h_blocks = ceil_div(state.horizontal_view_distance_voxels, mesh_block_size);
    let mesh_v_blocks = ceil_div(state.vertical_view_distance_voxels, mesh_block_size);
    let mesh_block_pos = floor_div_vec(state.local_position_voxels, mesh_block_size);
    state.mesh_box = Box3i::from_center_extents(
        mesh_block_pos,
        Vector3i::new(mesh_h_blocks, mesh_v_blocks, mesh_h_blocks),
    );

    // Data box is mesh box (in data-block units) padded by 1 for meshing
    // neighbours. factor == 1 here, so the conversion is identity.
    let data_h_blocks = mesh_h_blocks + 1;
    let data_v_blocks = mesh_v_blocks + 1;
    let data_block_pos = floor_div_vec(state.local_position_voxels, data_block_size);
    state.data_box = Box3i::from_center_extents(
        data_block_pos,
        Vector3i::new(data_h_blocks, data_v_blocks, data_h_blocks),
    );
}

fn ceil_div(a: i32, b: i32) -> i32 {
    (a + b - 1) / b
}

/// Compute per-LOD data/mesh boxes for a viewer. Each LOD level `N` uses a
/// block size of `data_block_size * (1 << N)`, so coarser LODs cover more world
/// space per block. The view distance (in voxels) is the same for all LODs —
/// the effect is that fewer, larger blocks are loaded at higher LODs. This is
/// the simplest multi-LOD strategy (the C++ clipbox system uses a per-LOD
/// distance falloff; this MVP uses uniform distance for simplicity).
fn compute_viewer_boxes_multi_lod(state: &mut ViewerState, data_block_size: i32, lod_count: u8) {
    if !state.requires_meshes {
        // No meshes: just keep data resident. Only LOD 0 for simplicity.
        let h_blocks = ceil_div(state.horizontal_view_distance_voxels, data_block_size);
        let v_blocks = ceil_div(state.vertical_view_distance_voxels, data_block_size);
        let block_pos = floor_div_vec(state.local_position_voxels, data_block_size);
        state.data_box =
            Box3i::from_center_extents(block_pos, Vector3i::new(h_blocks, v_blocks, h_blocks));
        state.mesh_box = Box3i::default();
        state.data_box_per_lod = vec![state.data_box];
        state.mesh_box_per_lod = vec![Box3i::default()];
        return;
    }

    state.data_box_per_lod = Vec::with_capacity(lod_count as usize);
    state.mesh_box_per_lod = Vec::with_capacity(lod_count as usize);

    for lod in 0..lod_count as i32 {
        let lod_block_size = data_block_size << lod;
        let mesh_h = ceil_div(state.horizontal_view_distance_voxels, lod_block_size);
        let mesh_v = ceil_div(state.vertical_view_distance_voxels, lod_block_size);
        let mesh_pos = floor_div_vec(state.local_position_voxels, lod_block_size);
        let mesh_box = Box3i::from_center_extents(mesh_pos, Vector3i::new(mesh_h, mesh_v, mesh_h));

        // Data box is mesh box padded by 1 (in this LOD's block units) for
        // meshing neighbours.
        let data_h = mesh_h + 1;
        let data_v = mesh_v + 1;
        let data_pos = floor_div_vec(state.local_position_voxels, lod_block_size);
        let data_box = Box3i::from_center_extents(data_pos, Vector3i::new(data_h, data_v, data_h));

        state.data_box_per_lod.push(data_box);
        state.mesh_box_per_lod.push(mesh_box);

        // LOD-0 backward compat fields.
        if lod == 0 {
            state.data_box = data_box;
            state.mesh_box = mesh_box;
        }
    }
}

fn floor_div_vec(v: Vector3i, b: i32) -> Vector3i {
    Vector3i::new(v.x.div_euclid(b), v.y.div_euclid(b), v.z.div_euclid(b))
}

fn block_box_to_voxel_box(block_box: Box3i, block_size: i32) -> Box3i {
    Box3i::new(block_box.position * block_size, block_box.size * block_size)
}

fn num_threads() -> usize {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    // Cap aggressively: terrain paging is latency-bound, not throughput-bound,
    // and the test suite spawns many cores. 4 keeps turn-around short.
    n.min(4)
}

// ---------------------------------------------------------------------------
// Task plumbing: downcast helpers for load/mesh task dispatch
// ---------------------------------------------------------------------------

/// Helper task the core spawns for each block load. Wraps a stream query into
/// the engine-agnostic `BlockDataOutput` shape so `apply_data_block_response`
/// can consume it.
struct LoadBlockForTerrainTask {
    position: Vector3i,
    lod_index: u8,
    data: Arc<SharedVoxelData>,
    stream: Arc<dyn VoxelStream>,
    output: Option<BlockDataOutput>,
}

impl LoadBlockForTerrainTask {
    fn new(
        position: Vector3i,
        lod_index: u8,
        data: Arc<SharedVoxelData>,
        stream: Arc<dyn VoxelStream>,
    ) -> Self {
        Self {
            position,
            lod_index,
            data,
            stream,
            output: None,
        }
    }
}

impl ThreadedTask for LoadBlockForTerrainTask {
    fn run(
        mut self: Box<Self>,
        _ctx: crate::tasks::ThreadedTaskContext,
    ) -> crate::tasks::TaskRunOutcome {
        // Try the stream first. If it has nothing, ask the generator.
        let settings = self.data.settings_snapshot();
        let bs = self.data.block_size() as i32;
        let format = settings.format;
        let generator = settings.generator;
        let lod = self.lod_index;
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(bs));
        format.configure_buffer(&mut voxels);
        let query = crate::streams::VoxelLoadQuery::new(&mut voxels, self.position, lod);
        match self.stream.load_voxel_block(query) {
            Ok(crate::streams::LoadResult::Found) => {
                self.output = Some(BlockDataOutput::loaded(self.position, lod, voxels, false));
            }
            Ok(crate::streams::LoadResult::NotFound) => {
                // Fall back to the generator if installed.
                if let Some(gen) = generator {
                    use crate::generators::base::VoxelQueryData;
                    let lod_stride = 1i32 << lod;
                    gen.generate_block(VoxelQueryData {
                        buffer: &mut voxels,
                        origin_in_voxels: self.position * bs * lod_stride,
                        lod: lod as u32,
                    });
                    self.output = Some(BlockDataOutput::loaded(self.position, lod, voxels, false));
                } else {
                    self.output = Some(BlockDataOutput::not_found(self.position, lod));
                }
            }
            Err(_err) => {
                self.output = Some(BlockDataOutput::loaded_dropped(self.position, lod));
            }
        }
        crate::tasks::TaskRunOutcome::Complete(self)
    }

    fn debug_name(&self) -> &'static str {
        "LoadBlockForTerrain"
    }
}

/// Helper: take a `Box<dyn ThreadedTask>` we know is a `LoadBlockForTerrainTask`
/// and apply its output. Using a trait-object downcast would require `Any`
/// bounds we don't want on `ThreadedTask`; we exploit the fact that
/// `drain_completed_tasks` already filtered by `debug_name`.
fn try_take_load_output(task: &mut dyn ThreadedTask) -> Option<BlockDataOutput> {
    let task = (task as &mut dyn std::any::Any).downcast_mut::<LoadBlockForTerrainTask>()?;
    task.output.take()
}

fn try_take_save_output(task: &mut dyn ThreadedTask) -> Option<BlockDataOutput> {
    let task = (task as &mut dyn std::any::Any).downcast_mut::<SaveBlockDataTask>()?;
    task.take_output()
}

fn try_take_mesh_output(mut task: Box<dyn ThreadedTask>) -> Option<BlockMeshOutput> {
    let task = (task.as_mut() as &mut dyn std::any::Any).downcast_mut::<MeshBlockTask>()?;
    task.take_output()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::MeshingDependency;
    use crate::generators::base::{GenResult, VoxelGenerator, VoxelQueryData};
    use crate::generators::simple::Flat;
    use crate::meshers::{MesherOutput, Surface, SurfaceArrays, VoxelMesher};
    use crate::storage::{ChannelId, VoxelData};
    use crate::streams::LoadResult;
    use crate::tasks::{TaskPriority, TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    /// A mesher that always emits one triangle, so we can tell from
    /// `mesh_blocks()[pos].is_loaded` whether the paging loop ran end-to-end.
    struct AlwaysOneTriangleMesher;
    impl VoxelMesher for AlwaysOneTriangleMesher {
        fn build(&self, output: &mut MesherOutput, _input: &crate::meshers::MesherInput<'_>) {
            let mut arrays = crate::meshers::transvoxel::structures::MeshArrays::default();
            let a = arrays.add_vertex(
                crate::math::Vector3f::zero(),
                crate::math::Vector3f::new(0.0, 1.0, 0.0),
                0,
                0,
                0,
                crate::math::Vector3f::zero(),
            );
            arrays.indices.extend_from_slice(&[a, a, a]);
            output
                .surfaces
                .push(Surface::new(SurfaceArrays::Transvoxel(arrays), 0));
        }
    }

    struct DebugNameCollisionTask;

    impl ThreadedTask for DebugNameCollisionTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            TaskRunOutcome::Complete(self)
        }

        fn debug_name(&self) -> &'static str {
            "MeshBlockTask"
        }
    }

    struct VoxelDataLockProbeGenerator {
        data: std::sync::Weak<SharedVoxelData>,
    }

    impl VoxelGenerator for VoxelDataLockProbeGenerator {
        fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
            let data = self.data.upgrade().expect("voxel data still alive");
            let guard = data
                .try_lock()
                .expect("VoxelData lock must be released before generator calls");
            drop(guard);

            input.buffer.clear_channel_f(ChannelId::Sdf.index(), -0.5);
            GenResult::default()
        }
    }

    struct VoxelDataLockProbeStream {
        data: std::sync::Weak<SharedVoxelData>,
    }

    impl VoxelStream for VoxelDataLockProbeStream {
        fn load_voxel_block(
            &self,
            _query: crate::streams::VoxelLoadQuery<'_>,
        ) -> crate::streams::StreamResult<LoadResult> {
            let data = self.data.upgrade().expect("voxel data still alive");
            let guard = data
                .try_lock()
                .expect("VoxelData lock must be released before stream calls");
            drop(guard);
            Ok(LoadResult::NotFound)
        }
    }

    struct SlowNotFoundStream {
        delay: Duration,
    }

    impl VoxelStream for SlowNotFoundStream {
        fn load_voxel_block(
            &self,
            _query: crate::streams::VoxelLoadQuery<'_>,
        ) -> crate::streams::StreamResult<LoadResult> {
            thread::sleep(self.delay);
            Ok(LoadResult::NotFound)
        }
    }

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
                return Err(crate::streams::VoxelStreamError::Io(
                    "injected save failure".into(),
                ));
            }
            self.inner
                .save_voxel_block(crate::streams::VoxelSaveQuery::new(
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

    fn build_core() -> VoxelTerrainCore {
        build_core_with_stream(Arc::new(MemoryStream::new()))
    }

    fn build_core_with_stream(stream: Arc<dyn VoxelStream>) -> VoxelTerrainCore {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::splat(-1024), Vector3i::splat(2048)));
        let flat = Flat {
            channel: ChannelId::Sdf,
            ..Flat::default()
        };
        let generator: Arc<dyn crate::generators::base::VoxelGenerator> = Arc::new(flat);
        data.set_generator(Some(generator));
        let mesher: Arc<dyn VoxelMesher> = Arc::new(AlwaysOneTriangleMesher);
        let meshing_dependency = MeshingDependency::new(mesher, None);
        VoxelTerrainCore::new(data, stream, meshing_dependency)
    }

    fn process_until<F>(
        core: &mut VoxelTerrainCore,
        viewers: &[ViewerUpdate],
        mut done: F,
    ) -> Vec<VoxelTerrainEvent>
    where
        F: FnMut(&VoxelTerrainCore, &[VoxelTerrainEvent]) -> bool,
    {
        let mut last_events = Vec::new();
        for _ in 0..100 {
            let events = core.process(viewers);
            if done(core, &events) {
                return events;
            }
            last_events = events;
            thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "terrain process condition was not reached; last events {last_events:?}, stats {:?}",
            core.stats
        );
    }

    #[test]
    fn process_does_not_wait_for_slow_load_tasks() {
        let stream: Arc<dyn VoxelStream> = Arc::new(SlowNotFoundStream {
            delay: Duration::from_millis(250),
        });
        let mut core = build_core_with_stream(stream);
        let bs = core.data_block_size();
        let viewers = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];

        let started = Instant::now();
        let _events = core.process(&viewers);
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(100),
            "process tick should only enqueue/drain tasks, elapsed {elapsed:?}"
        );
    }

    #[test]
    fn load_task_releases_data_lock_before_generator_fallback() {
        let data = Arc::new(SharedVoxelData::new(VoxelData::new()));
        data.set_generator(Some(Arc::new(VoxelDataLockProbeGenerator {
            data: Arc::downgrade(&data),
        })));

        let task =
            LoadBlockForTerrainTask::new(Vector3i::zero(), 0, data, Arc::new(MemoryStream::new()));
        let outcome = Box::new(task).run(ThreadedTaskContext::new(0, TaskPriority::max()));
        let mut completed = match outcome {
            TaskRunOutcome::Complete(task) => task,
            _ => panic!("load task must complete"),
        };
        let output = try_take_load_output(completed.as_mut()).expect("load output");

        assert!(!output.dropped);
    }

    #[test]
    fn load_task_releases_data_lock_before_stream_load() {
        let data = Arc::new(SharedVoxelData::new(VoxelData::new()));
        let stream: Arc<dyn VoxelStream> = Arc::new(VoxelDataLockProbeStream {
            data: Arc::downgrade(&data),
        });

        let task = LoadBlockForTerrainTask::new(Vector3i::zero(), 0, data, stream);
        let outcome = Box::new(task).run(ThreadedTaskContext::new(0, TaskPriority::max()));
        let mut completed = match outcome {
            TaskRunOutcome::Complete(task) => task,
            _ => panic!("load task must complete"),
        };
        let output = try_take_load_output(completed.as_mut()).expect("load output");

        assert!(!output.dropped);
        assert!(output.voxels.is_none());
    }

    #[test]
    fn paging_loads_and_meshes_blocks_around_a_viewer() {
        let mut core = build_core();
        let bs = core.data_block_size();

        // Place a viewer at the world origin with a small view distance. The
        // terrain should load the central data block and mesh the central
        // mesh block.
        let viewers = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];

        // Async ticks: the first call schedules loads, later calls drain load
        // results, schedule meshing, and drain mesh outputs.
        let events = process_until(&mut core, &viewers, |core, events| {
            events
                .iter()
                .any(|e| matches!(e, VoxelTerrainEvent::MeshBlockEntered(_)))
                && core
                    .mesh_blocks()
                    .get(&Vector3i::zero())
                    .is_some_and(|e| e.is_loaded)
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, VoxelTerrainEvent::MeshBlockEntered(_))),
            "expected a mesh to be produced, events were {events:?}"
        );
        assert!(core
            .mesh_blocks()
            .get(&Vector3i::zero())
            .is_some_and(|e| e.is_loaded));
        assert!(core.stats.blocks_loaded > 0);
    }

    #[test]
    fn paging_unloads_blocks_when_viewer_moves_away() {
        let mut core = build_core();
        let bs = core.data_block_size();

        let viewer_near = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];
        process_until(&mut core, &viewer_near, |core, _events| {
            core.mesh_blocks()
                .get(&Vector3i::zero())
                .is_some_and(|entry| entry.is_loaded)
        });
        let loaded_after_first = core.mesh_blocks().len();
        assert!(loaded_after_first > 0);

        // Move the viewer very far away (out of view distance). The mesh
        // block should unload on the next tick.
        let viewer_far = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::splat(bs * 100),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];
        let events = process_until(&mut core, &viewer_far, |_core, events| {
            events
                .iter()
                .any(|e| matches!(e, VoxelTerrainEvent::MeshBlockExited(_)))
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, VoxelTerrainEvent::MeshBlockExited(_))),
            "expected mesh blocks to exit, events were {events:?}"
        );
        // The origin block should no longer be tracked (viewer is far away).
        assert!(
            core.mesh_blocks().get(&Vector3i::zero()).is_none(),
            "expected origin block unloaded, mesh_map still has {:?}",
            core.mesh_blocks().keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn unloading_modified_data_block_saves_it_to_stream() {
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
        process_until(&mut core, &viewer, |core, _events| {
            core.data().block_snapshot(Vector3i::zero(), 0).is_some()
        });

        {
            let data = core.data();
            assert!(data.try_set_voxel(77, edited_voxel, channel));
            data.mark_area_modified(Box3i::new(edited_voxel, Vector3i::splat(1)), false);
        }

        let empty_viewers = Vec::new();
        process_until(&mut core, &empty_viewers, |_core, _events| {
            let mut loaded = VoxelBuffer::new(crate::storage::Allocator::Default);
            stream.load_block(Vector3i::zero(), 0, &mut loaded) == LoadResult::Found
                && loaded.get_voxel(1, 1, 1, channel) == 77
        });

        let mut loaded = VoxelBuffer::new(crate::storage::Allocator::Default);
        assert_eq!(
            stream.load_block(Vector3i::zero(), 0, &mut loaded),
            LoadResult::Found
        );
        assert_eq!(loaded.get_voxel(1, 1, 1, channel), 77);
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
        process_until(&mut core, &viewer, |core, _events| {
            core.data().block_snapshot(Vector3i::zero(), 0).is_some()
        });

        assert!(core.data().try_set_voxel(88, edited_voxel, channel));
        core.data()
            .mark_area_modified(Box3i::new(edited_voxel, Vector3i::splat(1)), false);

        let empty_viewers = Vec::new();
        process_until(&mut core, &empty_viewers, |_core, _events| {
            let mut loaded = VoxelBuffer::new(crate::storage::Allocator::Default);
            stream.load_block(Vector3i::zero(), 0, &mut loaded) == LoadResult::Found
                && loaded.get_voxel(1, 1, 1, channel) == 88
        });
    }

    #[test]
    fn stale_save_completion_not_matching_in_flight_generation_is_ignored() {
        let mut core = build_core();
        let key = SaveKey::new(Vector3i::zero(), 0);
        core.save_journal.insert(
            key,
            SaveJournalEntry {
                generation: 2,
                queued: false,
                in_flight_generation: Some(2),
                voxels: None,
                retry_count: 0,
            },
        );

        core.apply_save_response(BlockDataOutput::saved(Vector3i::zero(), 0, true, 1));

        let entry = core.save_journal.get(&key).expect("newer save must remain");
        assert_eq!(entry.generation, 2);
        assert_eq!(entry.in_flight_generation, Some(2));
        assert!(!entry.queued);
    }

    #[test]
    fn older_in_flight_completion_dispatches_queued_newer_save() {
        let mut core = build_core();
        let key = SaveKey::new(Vector3i::zero(), 0);
        core.save_journal.insert(
            key,
            SaveJournalEntry {
                generation: 2,
                queued: true,
                in_flight_generation: Some(1),
                voxels: Some(VoxelBuffer::with_size(Vector3i::splat(2))),
                retry_count: 0,
            },
        );

        core.apply_save_response(BlockDataOutput::saved(Vector3i::zero(), 0, true, 1));

        let entry = core
            .save_journal
            .get(&key)
            .expect("newer save should now be in flight");
        assert_eq!(entry.generation, 2);
        assert!(!entry.queued);
        assert_eq!(entry.in_flight_generation, Some(2));
        assert!(entry.voxels.is_none());
    }

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
        process_until(&mut core, &viewer, |core, _events| {
            core.data().block_snapshot(Vector3i::zero(), 0).is_some()
        });
        assert!(core.data().try_set_voxel(99, edited_voxel, channel));
        core.data()
            .mark_area_modified(Box3i::new(edited_voxel, Vector3i::splat(1)), false);
        core.process(&[]);

        core.shutdown_and_flush().unwrap();

        let mut loaded = VoxelBuffer::new(crate::storage::Allocator::Default);
        assert_eq!(
            stream.load_block(Vector3i::zero(), 0, &mut loaded),
            LoadResult::Found
        );
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
                in_flight_generation: None,
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

    #[test]
    fn loaded_blocks_keep_viewer_refs_from_coalesced_pending_loads() {
        let mut core = build_core();
        let bs = core.data_block_size();

        let two_viewers = vec![
            ViewerUpdate {
                id: 1,
                world_position_voxels: Vector3i::zero(),
                horizontal_view_distance_voxels: bs,
                vertical_view_distance_voxels: bs,
                requires_meshes: true,
            },
            ViewerUpdate {
                id: 2,
                world_position_voxels: Vector3i::zero(),
                horizontal_view_distance_voxels: bs,
                vertical_view_distance_voxels: bs,
                requires_meshes: true,
            },
        ];
        process_until(&mut core, &two_viewers, |core, _events| {
            core.data()
                .block_snapshot(Vector3i::zero(), 0)
                .is_some_and(|block| block.viewers.get() == 2)
        });

        let one_viewer = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];
        process_until(&mut core, &one_viewer, |core, _events| {
            core.data()
                .block_snapshot(Vector3i::zero(), 0)
                .is_some_and(|block| block.viewers.get() == 1)
        });

        let data = core.data();
        let origin_block = data
            .block_snapshot(Vector3i::zero(), 0)
            .expect("origin block should stay loaded while one viewer still references it");
        assert_eq!(origin_block.viewers.get(), 1);
    }

    #[test]
    fn mesh_task_output_downcast_rejects_debug_name_collision() {
        let task: Box<dyn ThreadedTask> = Box::new(DebugNameCollisionTask);
        assert!(try_take_mesh_output(task).is_none());
    }

    #[test]
    fn box_difference_returns_removing_slabs() {
        // Subtraction of a centred inner box from an outer box yields the
        // 6 surrounding slabs. Verify the helper used by paging diffs.
        let outer = Box3i::new(Vector3i::zero(), Vector3i::splat(4));
        let inner = Box3i::new(Vector3i::splat(1), Vector3i::splat(2));
        let slabs = outer.difference(inner);
        let cells_in_slabs: i64 = slabs
            .iter()
            .map(|b| (b.size.x as i64) * (b.size.y as i64) * (b.size.z as i64))
            .sum();
        // 4^3 - 2^3 = 56 cells outside the inner box.
        assert_eq!(cells_in_slabs, 64 - 8);

        // A non-overlapping subtraction returns the whole outer box.
        let disjoint = Box3i::new(Vector3i::splat(10), Vector3i::splat(1));
        assert_eq!(outer.difference(disjoint).len(), 1);
    }

    #[test]
    fn compute_viewer_boxes_pads_data_for_meshing_neighbours() {
        let mut state = ViewerState {
            local_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: 16,
            vertical_view_distance_voxels: 16,
            requires_meshes: true,
            ..ViewerState::default()
        };
        compute_viewer_boxes(&mut state, 16, 16);
        // ceil(16/16) = 1 block "radius"; from_center_extents produces a box
        // of size 2*1 = 2 per axis (center +/- 1, exclusive max).
        assert_eq!(state.mesh_box.size, Vector3i::splat(2));
        // Data box adds 1 block of padding for meshing neighbours (factor 1).
        assert!(state.data_box.size.x >= state.mesh_box.size.x);
    }

    // ---- M2.1 step 3: multi-LOD VoxelTerrainCore ----

    #[test]
    fn multi_lod_terrain_creates_correct_lod_count() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::splat(-512), Vector3i::splat(2048)));
        let mesher = Arc::new(crate::meshers::TransvoxelMesher::new());
        let dep = MeshingDependency::new(mesher, None);
        let core =
            VoxelTerrainCore::new_with_lod_count(data, Arc::new(MemoryStream::new()), dep, 3);
        assert_eq!(core.lod_count(), 3);
        // Each LOD has its own mesh map.
        assert_eq!(core.mesh_blocks_at_lod(0).len(), 0);
        assert_eq!(core.mesh_blocks_at_lod(1).len(), 0);
        assert_eq!(core.mesh_blocks_at_lod(2).len(), 0);
    }

    #[test]
    fn single_lod_terrain_backward_compat() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::splat(-512), Vector3i::splat(2048)));
        let mesher = Arc::new(crate::meshers::TransvoxelMesher::new());
        let dep = MeshingDependency::new(mesher, None);
        let core = VoxelTerrainCore::new_generator_only(data, dep);
        // Single-LOD: lod_count == 1, behaves identically to pre-M2.
        assert_eq!(core.lod_count(), 1);
        assert_eq!(core.mesh_blocks().len(), 0);
    }

    #[test]
    fn multi_lod_terrain_loads_blocks_at_both_lod_levels() {
        // End-to-end: a 2-LOD terrain with a viewer should produce mesh blocks
        // at both LOD 0 (fine) and LOD 1 (coarse, larger blocks).
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::splat(-512), Vector3i::splat(2048)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let mesher = Arc::new(crate::meshers::TransvoxelMesher::new());
        let dep = MeshingDependency::new(mesher, None);
        let mut core =
            VoxelTerrainCore::new_with_lod_count(data, Arc::new(MemoryStream::new()), dep, 2);
        assert_eq!(core.lod_count(), 2);

        // Viewer at origin with a small view distance.
        let viewers = vec![ViewerUpdate {
            id: 0,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: 48,
            vertical_view_distance_voxels: 48,
            requires_meshes: true,
        }];
        // Run several process ticks to let paging converge.
        for _ in 0..20 {
            core.process(&viewers);
        }
        // Both LOD levels should have at least some mesh blocks.
        let lod0_count = core.mesh_blocks_at_lod(0).len();
        let lod1_count = core.mesh_blocks_at_lod(1).len();
        assert!(
            lod0_count > 0,
            "LOD 0 should have mesh blocks, got {lod0_count}"
        );
        assert!(
            lod1_count > 0,
            "LOD 1 should have mesh blocks, got {lod1_count}"
        );
    }
}

// Keep the `VoxelBuffer` import used by the load task even though tests
// don't exercise it directly.
