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
use crate::meshers::{BlockMeshOutput, MeshBlockTask, MeshBlockTaskParams, MesherOutput};
use crate::storage::{BlockToSave, VoxelBuffer, VoxelData, VoxelDataBlock};
use crate::streams::{
    BlockDataOutput, BlockDataOutputKind, MemoryStream, SaveBlockDataTask, VoxelStream,
};
use crate::tasks::{ThreadedTask, ThreadedTaskRunner};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Lightweight viewer identity (mirrors C++ `ViewerID`).
pub type ViewerId = u32;

/// Per-viewer cached state used to diff boxes between frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewerState {
    pub local_position_voxels: Vector3i,
    /// Block-coord box of data blocks the viewer wants resident.
    pub data_box: Box3i,
    /// Block-coord box of mesh blocks the viewer wants rendered.
    pub mesh_box: Box3i,
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

/// One paired viewer specification, used to add or update viewers.
#[derive(Debug, Clone, Copy)]
pub struct ViewerUpdate {
    pub id: ViewerId,
    pub world_position_voxels: Vector3i,
    pub horizontal_view_distance_voxels: i32,
    pub vertical_view_distance_voxels: i32,
    pub requires_meshes: bool,
}

/// Engine-agnostic single-LOD paging terrain core.
pub struct VoxelTerrainCore {
    data: Arc<Mutex<VoxelData>>,
    mesh_map: HashMap<Vector3i, MeshBlockEntry>,
    paired_viewers: Vec<PairedViewer>,
    blocks_pending_load: Vec<Vector3i>,
    blocks_pending_update: Vec<Vector3i>,
    loading_blocks: HashMap<Vector3i, u32>,
    meshing_dependency: Arc<MeshingDependency>,
    stream: Arc<dyn VoxelStream>,
    task_runner: ThreadedTaskRunner,
    /// Maximum horizontal/vertical view distance the terrain will honour.
    /// Anything larger requested by a viewer is clamped.
    pub max_view_distance_voxels: i32,
    /// When `false`, no new loads/meshes are scheduled (matches C++
    /// `_automatic_loading_enabled`).
    pub automatic_loading_enabled: bool,
    pub stats: VoxelTerrainStats,
    /// Pending events waiting to be drained by the caller.
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
        let data = Arc::new(Mutex::new(data));
        let task_runner = ThreadedTaskRunner::new(num_threads());
        Self {
            data,
            mesh_map: HashMap::new(),
            paired_viewers: Vec::new(),
            blocks_pending_load: Vec::new(),
            blocks_pending_update: Vec::new(),
            loading_blocks: HashMap::new(),
            meshing_dependency,
            stream,
            task_runner,
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

    pub fn data(&self) -> Arc<Mutex<VoxelData>> {
        self.data.clone()
    }

    /// Returns the data block size (in voxels) used by the underlying
    /// `VoxelData`. The current port assumes mesh block size == data block
    /// size (factor 1).
    fn data_block_size(&self) -> i32 {
        // Lock just long enough to read the constant; matches the C++ inline
        // `get_data_block_size()`.
        let data = self.data.lock().expect("voxel data mutex poisoned");
        data.block_size() as i32
    }

    /// Reference to the underlying mesh-block hashmap (read-only). Tests and
    /// the future Godot binding use this to find blocks needing upload.
    pub fn mesh_blocks(&self) -> &HashMap<Vector3i, MeshBlockEntry> {
        &self.mesh_map
    }

    /// Synchronous entry point: pump viewer updates, run pending tasks, apply
    /// completed outputs. Returns the events emitted this tick.
    ///
    /// Pass the desired viewer set via `viewers`; any paired viewer not in
    /// the list is treated as removed (its boxes shrink to empty, triggering
    /// unloads). This mirrors the C++ `process_viewers` + `process_meshing`
    /// pair, plus the `apply_*_response` callbacks folded in (the Rust port
    /// runs tasks synchronously through the runner inside this call).
    pub fn process(&mut self, viewers: &[ViewerUpdate]) -> Vec<VoxelTerrainEvent> {
        self.events.clear();
        if !self.automatic_loading_enabled {
            return std::mem::take(&mut self.events);
        }

        self.process_viewers(viewers);
        // Run pending load + mesh tasks to completion before applying their
        // outputs. The C++ side lets them finish async across frames; the
        // Rust core drains them in-process for determinism (a real Godot
        // binding can swap this for a per-frame budget later).
        self.send_data_load_requests();
        self.task_runner.wait_for_all_tasks();
        self.drain_completed_tasks();

        self.process_meshing();
        self.task_runner.wait_for_all_tasks();
        self.drain_completed_tasks();

        std::mem::take(&mut self.events)
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
                // Viewer was removed: collapse boxes to empty so the diff
                // treats it as "everything went out of range".
                paired.prev_state = paired.state.clone();
                paired.state.data_box = Box3i::default();
                paired.state.mesh_box = Box3i::default();
                paired.state.requires_meshes = false;
                continue;
            }
            compute_viewer_boxes(&mut paired.state, data_block_size, mesh_block_size);
        }

        // Diff each viewer and apply view/unview operations. We collect the
        // ops into vectors first to avoid borrowing self mutably inside the
        // loop while iterating paired_viewers.
        let mut data_unview_boxes: Vec<(Box3i,)> = Vec::new();
        let mut data_view_boxes: Vec<(Box3i,)> = Vec::new();
        let mut mesh_unview_positions: Vec<Vector3i> = Vec::new();
        let mut mesh_view_positions: Vec<Vector3i> = Vec::new();

        for paired in self.paired_viewers.iter() {
            if paired.prev_state.data_box != paired.state.data_box {
                let removed = paired.prev_state.data_box.difference(paired.state.data_box);
                let added = paired.state.data_box.difference(paired.prev_state.data_box);
                for box_removed in removed {
                    data_unview_boxes.push((box_removed,));
                }
                for box_added in added {
                    data_view_boxes.push((box_added,));
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

        for (box_unviewed,) in data_unview_boxes {
            self.apply_data_unview(box_unviewed);
        }
        for (box_viewed,) in data_view_boxes {
            self.apply_data_view(box_viewed);
        }
        for pos in &mesh_unview_positions {
            self.unview_mesh_block(*pos);
        }
        for pos in &mesh_view_positions {
            self.view_mesh_block(*pos);
        }

        // Drop unpaired viewers from the list now that their boxes have
        // collapsed (matches the C++ swap-and-pop at the end of
        // process_viewers).
        self.paired_viewers.retain(|p| seen.contains(&p.id));
    }

    fn apply_data_view(&mut self, box_to_load: Box3i) {
        let mut missing = Vec::new();
        let mut found_positions = Vec::new();
        {
            let mut data = self.data.lock().expect("voxel data mutex poisoned");
            data.view_area(
                box_to_load,
                0,
                Some(&mut missing),
                Some(&mut found_positions),
                None,
            );
        }
        for bpos in missing {
            // Track loading viewers so duplicates coalesce and cancelled
            // loads can be re-requested.
            let entry = self.loading_blocks.entry(bpos).or_insert(0);
            *entry += 1;
            if *entry == 1 {
                self.blocks_pending_load.push(bpos);
            }
        }
        let _ = found_positions; // found blocks already resident; nothing to do
    }

    fn apply_data_unview(&mut self, box_to_unload: Box3i) {
        let mut removed_positions = Vec::new();
        let mut missing_positions = Vec::new();
        let mut saves = Vec::new();
        {
            let mut data = self.data.lock().expect("voxel data mutex poisoned");
            data.unview_area(
                box_to_unload,
                0,
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
            // Cancel any pending load for this block.
            self.loading_blocks.remove(&bpos);
            self.blocks_pending_load.retain(|p| *p != bpos);
        }
        for bpos in missing_positions {
            if let Some(count) = self.loading_blocks.get_mut(&bpos) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.loading_blocks.remove(&bpos);
                    self.blocks_pending_load.retain(|p| *p != bpos);
                }
            }
        }
    }

    fn enqueue_data_save(&mut self, save: BlockToSave) {
        let task = SaveBlockDataTask::new_voxels(
            save.position,
            save.lod_index,
            save.voxels,
            StreamingDependency::new(self.stream.clone()),
            None,
            false,
        );
        self.task_runner.enqueue(Box::new(task), false);
    }

    fn view_mesh_block(&mut self, bpos: Vector3i) {
        let entry = self.mesh_map.entry(bpos).or_insert_with(|| MeshBlockEntry {
            position: bpos,
            ..MeshBlockEntry::default()
        });
        entry.mesh_viewers += 1;
        // Try to schedule an immediate mesh update — if data is already
        // resident the mesh can produce geometry right away.
        self.try_schedule_mesh_update(bpos);
    }

    fn unview_mesh_block(&mut self, bpos: Vector3i) {
        let Some(entry) = self.mesh_map.get_mut(&bpos) else {
            return;
        };
        if entry.mesh_viewers > 0 {
            entry.mesh_viewers -= 1;
        }
        if entry.mesh_viewers == 0 {
            let was_loaded = entry.is_loaded;
            self.mesh_map.remove(&bpos);
            self.blocks_pending_update.retain(|p| *p != bpos);
            if was_loaded {
                self.events.push(VoxelTerrainEvent::MeshBlockExited(bpos));
            }
        }
    }

    /// `try_schedule_mesh_update`: if the data neighbourhood is resident,
    /// queue the mesh block for meshing. Matches C++ lines 435-462.
    fn try_schedule_mesh_update(&mut self, bpos: Vector3i) {
        let already = self
            .mesh_map
            .get(&bpos)
            .map(|e| e.is_in_update_list)
            .unwrap_or(false);
        if already {
            return;
        }
        let Some(entry) = self.mesh_map.get(&bpos) else {
            return;
        };
        if entry.mesh_viewers == 0 {
            return;
        }
        // factor == 1: data_box == mesh_block padded by 1.
        let data_box = Box3i::new(bpos, Vector3i::splat(1)).padded(1);
        let data = self.data.lock().expect("voxel data mutex poisoned");
        if !data.has_all_blocks_in_area(data_box, 0) {
            return;
        }
        let entry = self
            .mesh_map
            .get_mut(&bpos)
            .expect("mesh block exists after the early return");
        entry.is_in_update_list = true;
        self.blocks_pending_update.push(bpos);
    }

    /// After a data block loads, mesh blocks that touch it may now satisfy
    /// their neighbour-residency gate. Matches C++
    /// `try_schedule_mesh_update_from_data`.
    fn try_schedule_mesh_update_from_data(&mut self, voxel_box: Box3i) {
        // Pad by 1 so we catch mesh blocks whose data window overlaps the
        // newly-loaded block.
        let padded = voxel_box.padded(1);
        let data_block_size = self.data_block_size();
        let mesh_box = padded.downscaled(data_block_size);
        for bpos in mesh_box.iter_cells_zxy() {
            if self.mesh_map.contains_key(&bpos) {
                self.try_schedule_mesh_update(bpos);
            }
        }
    }

    fn send_data_load_requests(&mut self) {
        if self.blocks_pending_load.is_empty() {
            return;
        }
        let positions = std::mem::take(&mut self.blocks_pending_load);
        let data = self.data.clone();
        let stream = self.stream.clone();
        for bpos in positions {
            // Spawn a tiny task that loads the block from the stream and
            // hands the result back via `BlockDataOutput`. The runner stores
            // completed tasks; we drain them after the wait.
            let task = LoadBlockForTerrainTask::new(bpos, data.clone(), stream.clone());
            self.task_runner.enqueue(Box::new(task), false);
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
                self.apply_data_block_response(output);
            } else if let Some(output) = try_take_mesh_output(task) {
                self.apply_mesh_update(output);
            }
        }
    }

    fn process_meshing(&mut self) {
        if self.blocks_pending_update.is_empty() {
            return;
        }
        let positions = std::mem::take(&mut self.blocks_pending_update);
        let data = self.data.clone();
        let meshing_dependency = self.meshing_dependency.clone();
        for bpos in positions {
            // Reset the in-list flag now that the task is being dispatched
            // (the C++ side does this at line 1978 of process_meshing).
            if let Some(entry) = self.mesh_map.get_mut(&bpos) {
                entry.is_in_update_list = false;
            }
            let task = MeshBlockTask::new(MeshBlockTaskParams {
                position_in_blocks: bpos,
                lod_index: 0,
                data: data.clone(),
                meshing_dependency: meshing_dependency.clone(),
                collision_hint: false,
                lod_hint: false,
            });
            self.task_runner.enqueue(Box::new(task), false);
        }
    }

    /// Apply a data-load result (the C++ `apply_data_block_response`).
    pub fn apply_data_block_response(&mut self, output: BlockDataOutput) {
        let bpos = output.position_in_blocks;
        match output.kind {
            BlockDataOutputKind::Loaded | BlockDataOutputKind::NeedsGeneration => {
                if output.dropped {
                    // The load failed; if we still want it, re-request.
                    if self.loading_blocks.contains_key(&bpos) {
                        self.blocks_pending_load.push(bpos);
                    }
                    return;
                }
                let Some(voxels) = output.voxels else {
                    return;
                };
                let viewer_count = self.loading_blocks.remove(&bpos).unwrap_or(0);
                if viewer_count == 0 {
                    return;
                }
                let mut block = VoxelDataBlock::with_voxels(voxels, 0);
                for _ in 0..viewer_count {
                    block.viewers.add();
                }
                block.set_edited(true);
                let inserted = {
                    let mut data = self.data.lock().expect("voxel data mutex poisoned");
                    data.try_set_block(bpos, block)
                };
                if inserted {
                    self.stats.blocks_loaded += 1;
                    self.events.push(VoxelTerrainEvent::DataBlockLoaded(bpos));
                    let bs = self.data_block_size();
                    let voxel_box = Box3i::new(bpos * bs, Vector3i::splat(bs));
                    self.try_schedule_mesh_update_from_data(voxel_box);
                }
            }
            BlockDataOutputKind::NotFound => {
                // Treat as "no data here" — stop trying to load.
                self.loading_blocks.remove(&bpos);
            }
            BlockDataOutputKind::Saved => {}
        }
    }

    /// Apply a mesh result (the C++ `apply_mesh_update`).
    pub fn apply_mesh_update(&mut self, output: BlockMeshOutput) {
        let bpos = output.position_in_blocks;
        if output.dropped {
            self.stats.meshes_dropped += 1;
            return;
        }
        let Some(entry) = self.mesh_map.get_mut(&bpos) else {
            // Block was unloaded between dispatch and completion.
            self.stats.meshes_dropped += 1;
            return;
        };
        let became_loaded = !entry.is_loaded && !output.surfaces.is_empty();
        entry.output = Some(output.surfaces);
        entry.is_loaded = true;
        self.stats.meshes_built += 1;
        if became_loaded {
            self.events.push(VoxelTerrainEvent::MeshBlockEntered(bpos));
        }
    }
}

/// Box-diff helper: returns the sub-boxes of `self` not covered by `other`.
/// Used to compute "what went out of range" between two frames.
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

fn floor_div_vec(v: Vector3i, b: i32) -> Vector3i {
    Vector3i::new(v.x.div_euclid(b), v.y.div_euclid(b), v.z.div_euclid(b))
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
    data: Arc<Mutex<VoxelData>>,
    stream: Arc<dyn VoxelStream>,
    output: Option<BlockDataOutput>,
}

impl LoadBlockForTerrainTask {
    fn new(position: Vector3i, data: Arc<Mutex<VoxelData>>, stream: Arc<dyn VoxelStream>) -> Self {
        Self {
            position,
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
        let bs = self
            .data
            .lock()
            .expect("voxel data mutex poisoned")
            .block_size() as i32;
        let format = self
            .data
            .lock()
            .expect("voxel data mutex poisoned")
            .format();
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(bs));
        format.configure_buffer(&mut voxels);
        let query = crate::streams::VoxelLoadQuery::new(&mut voxels, self.position, 0);
        match self.stream.load_voxel_block(query) {
            Ok(crate::streams::LoadResult::Found) => {
                self.output = Some(BlockDataOutput::loaded(self.position, 0, voxels, false));
            }
            Ok(crate::streams::LoadResult::NotFound) => {
                // Fall back to the generator if installed.
                let data = self.data.lock().expect("voxel data mutex poisoned");
                if let Some(gen) = data.generator() {
                    use crate::generators::base::VoxelQueryData;
                    gen.generate_block(VoxelQueryData {
                        buffer: &mut voxels,
                        origin_in_voxels: self.position * bs,
                        lod: 0,
                    });
                    drop(data);
                    self.output = Some(BlockDataOutput::loaded(self.position, 0, voxels, false));
                } else {
                    drop(data);
                    self.output = Some(BlockDataOutput::not_found(self.position, 0));
                }
            }
            Err(_err) => {
                self.output = Some(BlockDataOutput::loaded_dropped(self.position, 0));
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
    use crate::generators::simple::Flat;
    use crate::meshers::{MesherOutput, Surface, SurfaceArrays, VoxelMesher};
    use crate::storage::{ChannelId, VoxelData};
    use crate::streams::LoadResult;
    use crate::tasks::{TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
    use std::sync::Mutex;

    /// A mesher that always emits one triangle, so we can tell from
    /// `mesh_blocks()[pos].is_loaded` whether the paging loop ran end-to-end.
    struct AlwaysOneTriangleMesher;
    impl VoxelMesher for AlwaysOneTriangleMesher {
        fn build(&mut self, output: &mut MesherOutput, _input: &crate::meshers::MesherInput<'_>) {
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
        let mesher: Arc<Mutex<Box<dyn VoxelMesher>>> =
            Arc::new(Mutex::new(Box::new(AlwaysOneTriangleMesher)));
        let meshing_dependency = MeshingDependency::new(mesher, None);
        VoxelTerrainCore::new(data, stream, meshing_dependency)
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

        // First tick: schedules loads + mesh task; tasks run synchronously
        // inside `process`. The central mesh block should be loaded by the
        // end of this call.
        let events = core.process(&viewers);
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
        core.process(&viewer_near);
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
        let events = core.process(&viewer_far);
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
        core.process(&viewer);

        {
            let data = core.data();
            let mut data = data.lock().expect("voxel data mutex poisoned");
            assert!(data.try_set_voxel(77, edited_voxel, channel));
            data.mark_area_modified(Box3i::new(edited_voxel, Vector3i::splat(1)), false);
        }

        core.process(&[]);

        let mut loaded = VoxelBuffer::new(crate::storage::Allocator::Default);
        assert_eq!(
            stream.load_block(Vector3i::zero(), 0, &mut loaded),
            LoadResult::Found
        );
        assert_eq!(loaded.get_voxel(1, 1, 1, channel), 77);
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
        core.process(&two_viewers);

        let one_viewer = vec![ViewerUpdate {
            id: 1,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: bs,
            vertical_view_distance_voxels: bs,
            requires_meshes: true,
        }];
        core.process(&one_viewer);

        let data = core.data();
        let data = data.lock().expect("voxel data mutex poisoned");
        let origin_block = data
            .get_block(Vector3i::zero(), 0)
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
}

// Keep the `VoxelBuffer` import used by the load task even though tests
// don't exercise it directly.
