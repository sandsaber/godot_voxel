//! Threaded mesh block task — the engine-agnostic algorithm core.
//!
//! Ported from `meshers/mesh_block_task.{h,cpp}` minus Godot bindings
//! (`Ref<Mesh>`, `ArrayMesh`, GPU/detail-texture paths, `VoxelEngine`
//! callback dispatch). Implements the same pipeline the C++ terrain runs on
//! worker threads:
//!
//! 1. [`gather_voxels_cpu`] — gather the central data block plus its 3×3×3
//!    neighbours into a padded `VoxelBuffer`. Missing neighbours are filled
//!    by the installed generator (the same contract as C++
//!    `copy_block_and_neighbors` with `out_boxes_to_generate = nullptr`).
//! 2. [`MeshBlockTask::run`] — calls the configured [`VoxelMesher`] against
//!    the gathered voxels and stores the [`MesherOutput`].
//!
//! The current port assumes `mesh_block_size_factor == 1` (one mesh block
//! covers exactly one data block), which matches `VoxelTerrain`. The
//! general multi-block case used by `VoxelLodTerrain` is a follow-up.

use crate::engine::MeshingDependency;
use crate::generators::base::{VoxelGenerator, VoxelQueryData};
use crate::math::{Box3i, Vector3i};
use crate::meshers::{MesherInput, MesherOutput, VoxelMesher};
use crate::storage::{SharedVoxelData, VoxelBuffer, VoxelData};
use crate::tasks::{TaskPriority, TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
use std::sync::Arc;

/// Output of a [`MeshBlockTask`]. Mirrors the C++ `VoxelEngine::BlockMeshOutput`
/// minus the `Ref<Mesh>` field — surfaces are kept as native Rust
/// [`MesherOutput`] so callers (the future `VoxelEngine` port, or a Godot
/// binding in `voxel-gdext`) can upload them however they like.
#[derive(Debug, Default)]
pub struct BlockMeshOutput {
    /// World-space block position (block coordinates at the task's LOD).
    pub position_in_blocks: Vector3i,
    /// LOD index the task ran at.
    pub lod_index: u8,
    /// Surfaces produced by the mesher.
    pub surfaces: MesherOutput,
    /// `true` when the task ran but its dependency was invalidated before
    /// the result could be applied (caller should drop the output).
    pub dropped: bool,
}

/// Configuration for [`MeshBlockTask::new`].
#[derive(Clone)]
pub struct MeshBlockTaskParams {
    pub position_in_blocks: Vector3i,
    pub lod_index: u8,
    pub data: Arc<SharedVoxelData>,
    pub meshing_dependency: Arc<MeshingDependency>,
    /// Hint that collision geometry is wanted (passed to the mesher).
    pub collision_hint: bool,
    /// Hint that the mesh will be used in a variable-LOD context.
    pub lod_hint: bool,
}

/// Threaded task: gathers voxels and runs a [`VoxelMesher`] against them.
///
/// Ported from C++ `MeshBlockTask`. Synchronous CPU-only path: no GPU
/// generation, no detail-texture baking, no `VoxelEngine` callback dispatch.
/// Callers drain [`BlockMeshOutput`] via [`MeshBlockTask::take_output`].
pub struct MeshBlockTask {
    position_in_blocks: Vector3i,
    lod_index: u8,
    data: Arc<SharedVoxelData>,
    meshing_dependency: Arc<MeshingDependency>,
    collision_hint: bool,
    lod_hint: bool,
    has_run: bool,
    output: Option<BlockMeshOutput>,
}

impl MeshBlockTask {
    pub fn new(params: MeshBlockTaskParams) -> Self {
        Self {
            position_in_blocks: params.position_in_blocks,
            lod_index: params.lod_index,
            data: params.data,
            meshing_dependency: params.meshing_dependency,
            collision_hint: params.collision_hint,
            lod_hint: params.lod_hint,
            has_run: false,
            output: None,
        }
    }

    pub const fn position_in_blocks(&self) -> Vector3i {
        self.position_in_blocks
    }

    pub const fn lod_index(&self) -> u8 {
        self.lod_index
    }

    pub const fn has_run(&self) -> bool {
        self.has_run
    }

    pub fn take_output(&mut self) -> Option<BlockMeshOutput> {
        self.output.take()
    }

    /// Run the gather+mesh pipeline synchronously. Equivalent to the C++
    /// `MeshBlockTask::run` CPU branch (gather_voxels_cpu + build_mesh).
    pub fn run_meshing(&mut self) {
        self.output = None;

        if !self.meshing_dependency.is_valid() {
            // The terrain swapped the mesher/generator mid-flight; emit a
            // dropped output so the caller knows to requeue.
            self.has_run = true;
            self.output = Some(BlockMeshOutput {
                position_in_blocks: self.position_in_blocks,
                lod_index: self.lod_index,
                surfaces: MesherOutput::default(),
                dropped: true,
            });
            return;
        }

        let mesher_handle = self.meshing_dependency.mesher();
        let mesher: &dyn VoxelMesher = mesher_handle.as_ref();
        let min_padding = mesher.minimum_padding() as i32;
        let max_padding = mesher.maximum_padding() as i32;
        let channels_mask = mesher.used_channels_mask();

        let generator_handle = self.meshing_dependency.generator();

        let data_block_size = self.data.block_size() as i32;
        let lod_block_size = data_block_size << u32::from(self.lod_index);
        let read_box = Box3i::new(
            (self.position_in_blocks - Vector3i::splat(1)) * lod_block_size,
            Vector3i::splat(lod_block_size * 3),
        );

        let mut voxels = VoxelBuffer::with_size(Vector3i::zero());
        let gather_plan = {
            let _read_region = self.data.read_region(self.lod_index as usize, read_box);
            gather_voxels_cpu_shared_snapshot(
                &mut voxels,
                min_padding,
                max_padding,
                channels_mask,
                generator_handle.is_some(),
                &self.data,
                self.lod_index,
                self.position_in_blocks,
            )
        };
        if let Some(generator) = generator_handle.as_deref() {
            generate_missing_voxel_regions(&mut voxels, generator, &gather_plan, self.lod_index);
        }

        // Build the mesh. The padded buffer is what the mesher sees; the
        // origin reported to the mesher is the world-space corner of the
        // *unpadded* mesh block (matching C++ build_mesh).
        let mesh_block_size = voxels.size() - Vector3i::splat(min_padding + max_padding);
        let block_world_origin =
            self.position_in_blocks * (mesh_block_size << u32::from(self.lod_index));

        let mut surfaces = MesherOutput::default();
        let input = MesherInput {
            voxels: &voxels,
            generator: generator_handle.as_deref(),
            origin_in_voxels: block_world_origin,
            lod_index: self.lod_index,
            collision_hint: self.collision_hint,
            lod_hint: self.lod_hint,
        };
        mesher.build(&mut surfaces, &input);

        self.has_run = true;
        self.output = Some(BlockMeshOutput {
            position_in_blocks: self.position_in_blocks,
            lod_index: self.lod_index,
            surfaces,
            dropped: false,
        });
    }
}

impl ThreadedTask for MeshBlockTask {
    fn run(mut self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
        self.run_meshing();
        TaskRunOutcome::Complete(self)
    }

    fn priority(&mut self) -> TaskPriority {
        // Mesh tasks are higher-priority than stream tasks but lower than
        // main-thread work; the C++ side uses TASK_PRIORITY_MESH_BAND2 in
        // band 2 via PriorityDependency. The bare constant lives in
        // constants::voxel_constants; we keep a sensible default here until
        // the priority-dependency wiring moves into this task.
        TaskPriority::new(0, 0, 0, 0)
    }

    fn is_cancelled(&mut self) -> bool {
        !self.meshing_dependency.is_valid()
    }

    fn debug_name(&self) -> &'static str {
        "MeshBlockTask"
    }
}

/// Gathers a 3×3×3 neighbourhood of data blocks into a padded `dst` buffer.
///
/// Ported from C++ `copy_block_and_neighbors` for `mesh_block_size_factor == 1`.
/// `dst` is configured to `(block_size + min_padding + max_padding)³` with the
/// caller's [`VoxelFormat`], matching C++ which configures the buffer before
/// copying/generating the neighbour regions. The function:
///
/// 1. Copies each neighbour's channel data into the matching sub-region of
///    `dst` (skipping empty/missing blocks).
/// 2. For missing blocks inside the volume bounds, runs `generator` to fill
///    the corresponding region of `dst` directly.
///
/// Returns the world-space origin of the *padded* buffer (the corner of the
/// padding halo, not the central block). This matches the C++
/// `origin_in_voxels` out-parameter.
#[allow(clippy::too_many_arguments)]
pub fn gather_voxels_cpu(
    dst: &mut VoxelBuffer,
    min_padding: i32,
    max_padding: i32,
    channels_mask: u32,
    generator: Option<&dyn VoxelGenerator>,
    voxel_data: &VoxelData,
    lod_index: u8,
    mesh_block_pos: Vector3i,
) -> Vector3i {
    let gather_plan = gather_voxels_cpu_snapshot(
        dst,
        min_padding,
        max_padding,
        channels_mask,
        generator.is_some(),
        voxel_data,
        lod_index,
        mesh_block_pos,
    );
    if let Some(generator) = generator {
        generate_missing_voxel_regions(dst, generator, &gather_plan, lod_index);
    }
    gather_plan.origin_in_voxels
}

#[derive(Debug, Clone, Copy)]
struct MissingVoxelRegion {
    dst_offset: Vector3i,
    origin_in_voxels: Vector3i,
}

#[derive(Debug)]
struct GatherVoxelPlan {
    origin_in_voxels: Vector3i,
    format: crate::storage::VoxelFormat,
    data_block_size: i32,
    channels: Vec<usize>,
    missing_regions: Vec<MissingVoxelRegion>,
}

#[allow(clippy::too_many_arguments)]
fn gather_voxels_cpu_snapshot(
    dst: &mut VoxelBuffer,
    min_padding: i32,
    max_padding: i32,
    channels_mask: u32,
    queue_missing_regions: bool,
    voxel_data: &VoxelData,
    lod_index: u8,
    mesh_block_pos: Vector3i,
) -> GatherVoxelPlan {
    let data_block_size = voxel_data.block_size() as i32;
    let mesh_block_size = data_block_size; // factor == 1
    let padded_size = mesh_block_size + min_padding + max_padding;
    let format = voxel_data.format();

    // (Re)create `dst` at the padded size and configure channels. The C++
    // path calls `dst.create(size, &format)`; our caller already configured
    // the format, so we just resize.
    if dst.size() != Vector3i::splat(padded_size) {
        *dst = VoxelBuffer::with_size(Vector3i::splat(padded_size));
    }
    format.configure_buffer(dst);

    let channels: Vec<usize> = (0..8u32)
        .filter(|ci| (channels_mask & (1u32 << ci)) != 0)
        .map(|ci| ci as usize)
        .collect();
    let mut missing_regions = Vec::new();

    // Padded buffer's world origin (corner of the halo, not the block).
    let origin_in_voxels_without_padding = mesh_block_pos * mesh_block_size;
    let origin_in_voxels = origin_in_voxels_without_padding - Vector3i::splat(min_padding);

    // Each neighbour occupies a `data_block_size`³ slab of `dst`. Iterate
    // ZXY (matching C++) and compute the source offset into `dst`.
    for dz in -1..=1 {
        for dx in -1..=1 {
            for dy in -1..=1 {
                let neighbour_block_pos = mesh_block_pos + Vector3i::new(dx, dy, dz);
                let dst_offset =
                    Vector3i::new(dx, dy, dz) * data_block_size + Vector3i::splat(min_padding);

                let neighbour_present = voxel_data
                    .get_block(neighbour_block_pos, lod_index as usize)
                    .is_some_and(|block| block.has_voxels());

                if neighbour_present {
                    let src = voxel_data
                        .get_block(neighbour_block_pos, lod_index as usize)
                        .unwrap()
                        .voxels();
                    for &channel_index in &channels {
                        dst.copy_channel_from_area(
                            src,
                            Vector3i::zero(),
                            src.size(),
                            dst_offset,
                            channel_index,
                        );
                    }
                } else if queue_missing_regions {
                    // Missing neighbour inside bounds: queue the generator
                    // work so callers holding VoxelData's lock can drop it
                    // before running the heavy generator.
                    let neighbour_origin =
                        (neighbour_block_pos * data_block_size) << u32::from(lod_index);
                    missing_regions.push(MissingVoxelRegion {
                        dst_offset,
                        origin_in_voxels: neighbour_origin,
                    });
                }
                // Else: no generator and missing block — `dst` keeps the
                // format default for that region (matches C++ behaviour
                // when no generator is installed).
            }
        }
    }

    GatherVoxelPlan {
        origin_in_voxels,
        format,
        data_block_size,
        channels,
        missing_regions,
    }
}

#[allow(clippy::too_many_arguments)]
fn gather_voxels_cpu_shared_snapshot(
    dst: &mut VoxelBuffer,
    min_padding: i32,
    max_padding: i32,
    channels_mask: u32,
    queue_missing_regions: bool,
    voxel_data: &SharedVoxelData,
    lod_index: u8,
    mesh_block_pos: Vector3i,
) -> GatherVoxelPlan {
    let data_block_size = voxel_data.block_size() as i32;
    let mesh_block_size = data_block_size; // factor == 1
    let padded_size = mesh_block_size + min_padding + max_padding;
    let format = voxel_data.format();

    if dst.size() != Vector3i::splat(padded_size) {
        *dst = VoxelBuffer::with_size(Vector3i::splat(padded_size));
    }
    format.configure_buffer(dst);

    let channels: Vec<usize> = (0..8u32)
        .filter(|ci| (channels_mask & (1u32 << ci)) != 0)
        .map(|ci| ci as usize)
        .collect();
    let mut missing_regions = Vec::new();

    let origin_in_voxels_without_padding = mesh_block_pos * mesh_block_size;
    let origin_in_voxels = origin_in_voxels_without_padding - Vector3i::splat(min_padding);

    let lod_loaded = usize::from(lod_index) < voxel_data.lod_count();
    let mut visit_neighbour = |neighbour_block_pos: Vector3i,
                               dst_offset: Vector3i,
                               src: Option<&VoxelBuffer>| {
        if let Some(src) = src {
            for &channel_index in &channels {
                dst.copy_channel_from_area(
                    src,
                    Vector3i::zero(),
                    src.size(),
                    dst_offset,
                    channel_index,
                );
            }
        } else if queue_missing_regions {
            let neighbour_origin = (neighbour_block_pos * data_block_size) << u32::from(lod_index);
            missing_regions.push(MissingVoxelRegion {
                dst_offset,
                origin_in_voxels: neighbour_origin,
            });
        }
    };

    if lod_loaded {
        voxel_data.with_lod_map(lod_index as usize, |map| {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let neighbour_block_pos = mesh_block_pos + Vector3i::new(dx, dy, dz);
                        let dst_offset = Vector3i::new(dx, dy, dz) * data_block_size
                            + Vector3i::splat(min_padding);

                        let src = map
                            .get_block(neighbour_block_pos)
                            .filter(|block| block.has_voxels())
                            .map(|block| block.voxels());
                        visit_neighbour(neighbour_block_pos, dst_offset, src);
                    }
                }
            }
        });
    } else {
        for dz in -1..=1 {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let neighbour_block_pos = mesh_block_pos + Vector3i::new(dx, dy, dz);
                    let dst_offset =
                        Vector3i::new(dx, dy, dz) * data_block_size + Vector3i::splat(min_padding);

                    visit_neighbour(neighbour_block_pos, dst_offset, None);
                }
            }
        }
    }

    GatherVoxelPlan {
        origin_in_voxels,
        format,
        data_block_size,
        channels,
        missing_regions,
    }
}

fn generate_missing_voxel_regions(
    dst: &mut VoxelBuffer,
    generator: &dyn VoxelGenerator,
    gather_plan: &GatherVoxelPlan,
    lod_index: u8,
) {
    for region in &gather_plan.missing_regions {
        // The generator expects a standalone block-sized buffer. Copy the
        // requested channels back into the padded mesh buffer afterwards.
        let mut scratch = VoxelBuffer::with_size(Vector3i::splat(gather_plan.data_block_size));
        gather_plan.format.configure_buffer(&mut scratch);
        generator.generate_block(VoxelQueryData {
            buffer: &mut scratch,
            origin_in_voxels: region.origin_in_voxels,
            lod: lod_index as u32,
        });
        for &channel_index in &gather_plan.channels {
            dst.copy_channel_from_area(
                &scratch,
                Vector3i::zero(),
                scratch.size(),
                region.dst_offset,
                channel_index,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{gather_voxels_cpu, MeshBlockTask, MeshBlockTaskParams};
    use crate::engine::MeshingDependency;
    use crate::generators::base::{GenResult, VoxelGenerator, VoxelQueryData};
    use crate::math::{Box3i, Vector3i};
    use crate::meshers::{MesherInput, MesherOutput, Surface, SurfaceArrays, VoxelMesher};
    use crate::storage::{ChannelId, SharedVoxelData, VoxelBuffer, VoxelData};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex, Weak};
    use std::time::{Duration, Instant};

    /// A generator that writes a constant raw value into the SDF channel of
    /// every voxel it sees. Lets us verify the gather step fills missing
    /// neighbours with generator output.
    struct ConstantSdfGenerator {
        value: f32,
    }
    impl VoxelGenerator for ConstantSdfGenerator {
        fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
            input
                .buffer
                .clear_channel_f(ChannelId::Sdf.index(), self.value);
            GenResult::default()
        }
    }

    struct VoxelDataLockProbeGenerator {
        data: Weak<SharedVoxelData>,
    }

    impl VoxelGenerator for VoxelDataLockProbeGenerator {
        fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
            let data = self.data.upgrade().expect("voxel data still alive");
            let guard = data
                .try_lock()
                .expect("VoxelData lock must be released before generator calls");
            drop(guard);

            input.buffer.clear_channel_f(ChannelId::Sdf.index(), -0.25);
            GenResult::default()
        }
    }

    struct VoxelDataLockProbeMesher {
        data: Weak<SharedVoxelData>,
        build_calls: Arc<Mutex<usize>>,
    }

    impl VoxelMesher for VoxelDataLockProbeMesher {
        fn build(&self, _output: &mut MesherOutput, _input: &MesherInput<'_>) {
            let data = self.data.upgrade().expect("voxel data still alive");
            let guard = data
                .try_lock()
                .expect("VoxelData lock must be released before mesher calls");
            drop(guard);
            *self.build_calls.lock().unwrap() += 1;
        }

        fn used_channels_mask(&self) -> u32 {
            1 << ChannelId::Sdf.index()
        }
    }

    struct OverlapProbeMesher {
        entered: Arc<(Mutex<usize>, Condvar)>,
        inside: Arc<AtomicUsize>,
        max_inside: Arc<AtomicUsize>,
    }

    struct InsideBuildGuard<'a>(&'a AtomicUsize);

    impl Drop for InsideBuildGuard<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl VoxelMesher for OverlapProbeMesher {
        fn build(&self, _output: &mut MesherOutput, _input: &MesherInput<'_>) {
            let current = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
            let _guard = InsideBuildGuard(&self.inside);
            self.max_inside.fetch_max(current, Ordering::SeqCst);

            let (lock, cvar) = &*self.entered;
            let mut entered = lock.lock().unwrap();
            *entered += 1;
            cvar.notify_all();

            let deadline = Instant::now() + Duration::from_secs(2);
            while *entered < 2 {
                let now = Instant::now();
                assert!(
                    now < deadline,
                    "mesh tasks did not overlap inside the shared mesher"
                );
                let timeout = deadline.saturating_duration_since(now);
                let (next, wait) = cvar.wait_timeout(entered, timeout).unwrap();
                entered = next;
                assert!(
                    !wait.timed_out() || *entered >= 2,
                    "mesh tasks did not overlap inside the shared mesher"
                );
            }
        }

        fn used_channels_mask(&self) -> u32 {
            0
        }
    }

    /// A mesher that emits a single transvoxel surface with one dummy
    /// triangle, proving the gather→build pipeline ran end-to-end.
    struct DummyMesher {
        build_calls: Arc<Mutex<usize>>,
    }
    impl VoxelMesher for DummyMesher {
        fn build(&self, output: &mut MesherOutput, _input: &MesherInput<'_>) {
            *self.build_calls.lock().unwrap() += 1;
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

        fn used_channels_mask(&self) -> u32 {
            1 << ChannelId::Sdf.index()
        }
    }

    fn shared_data_with_central_block(block_size: i32) -> Arc<SharedVoxelData> {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(
            Vector3i::splat(-block_size * 4),
            Vector3i::splat(block_size * 8),
        ));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        // Force-residence of the central block by writing one voxel into it.
        data.try_set_voxel(1, Vector3i::new(1, 1, 1), ChannelId::Type.index());
        Arc::new(SharedVoxelData::new(data))
    }

    #[test]
    fn gather_voxels_cpu_fills_padded_buffer_from_central_block_and_generates_neighbours() {
        let mut data = VoxelData::new();
        let bs = data.block_size() as i32;
        data.set_bounds(Box3i::new(
            Vector3i::splat(-bs * 4),
            Vector3i::splat(bs * 8),
        ));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        // Materialise the central block with a recognisable raw value.
        data.try_set_voxel(7, Vector3i::new(1, 1, 1), ChannelId::Type.index());

        let generator = ConstantSdfGenerator { value: -0.5 };

        let mut dst = VoxelBuffer::with_size(Vector3i::zero());
        gather_voxels_cpu(
            &mut dst,
            1,
            1,
            1u32 << ChannelId::Type.index() | 1u32 << ChannelId::Sdf.index(),
            Some(&generator),
            &data,
            0,
            Vector3i::zero(),
        );

        // Padded size is bs + 2 (1+1 padding on every axis).
        assert_eq!(dst.size(), Vector3i::splat(bs + 2));
        // The central block wrote Type=7 at local (1,1,1) world; in dst that
        // voxel sits at (min_padding + 1, min_padding + 1, min_padding + 1).
        assert_eq!(dst.get_voxel(2, 2, 2, ChannelId::Type.index()), 7);
        // A voxel in a missing neighbour region (just outside the central
        // block) was filled by the generator with a near -0.5 SDF. The exact
        // value runs through the SDF channel's signed-normalised quantiser
        // (default 16-bit), so check the sign and rough magnitude instead of
        // an exact equality.
        let sdf = dst.get_voxel_f(0, 0, 0, ChannelId::Sdf.index());
        assert!(sdf < 0.0, "expected negative SDF, got {sdf}");
        assert!(
            (sdf - (-0.5)).abs() < 0.05,
            "expected SDF near -0.5, got {sdf}"
        );
    }

    #[test]
    fn mesh_block_task_produces_non_empty_output_for_resident_central_block() {
        let data = shared_data_with_central_block(16);
        let build_calls = Arc::new(Mutex::new(0));
        let mesher: Arc<dyn VoxelMesher> = Arc::new(DummyMesher {
            build_calls: build_calls.clone(),
        });
        let generator: Arc<dyn VoxelGenerator> = Arc::new(ConstantSdfGenerator { value: -1.0 });
        let meshing_dep = MeshingDependency::new(mesher, Some(generator));

        let mut task = MeshBlockTask::new(MeshBlockTaskParams {
            position_in_blocks: Vector3i::zero(),
            lod_index: 0,
            data: data.clone(),
            meshing_dependency: meshing_dep,
            collision_hint: false,
            lod_hint: false,
        });

        task.run_meshing();
        let output = task.take_output().expect("task produced output");

        assert!(!output.dropped);
        assert_eq!(output.position_in_blocks, Vector3i::zero());
        assert_eq!(output.lod_index, 0);
        assert!(output.surfaces.total_triangle_count() > 0);
        assert_eq!(*build_calls.lock().unwrap(), 1);
    }

    #[test]
    fn mesh_block_task_releases_data_lock_before_generator_fallback() {
        let data = shared_data_with_central_block(16);
        let build_calls = Arc::new(Mutex::new(0));
        let mesher: Arc<dyn VoxelMesher> = Arc::new(DummyMesher {
            build_calls: build_calls.clone(),
        });
        let generator: Arc<dyn VoxelGenerator> = Arc::new(VoxelDataLockProbeGenerator {
            data: Arc::downgrade(&data),
        });
        let meshing_dep = MeshingDependency::new(mesher, Some(generator));

        let mut task = MeshBlockTask::new(MeshBlockTaskParams {
            position_in_blocks: Vector3i::zero(),
            lod_index: 0,
            data,
            meshing_dependency: meshing_dep,
            collision_hint: false,
            lod_hint: false,
        });

        task.run_meshing();
        let output = task.take_output().expect("task produced output");

        assert!(!output.dropped);
        assert_eq!(*build_calls.lock().unwrap(), 1);
    }

    #[test]
    fn mesh_block_task_releases_data_lock_before_mesher_build() {
        let data = shared_data_with_central_block(16);
        let build_calls = Arc::new(Mutex::new(0));
        let mesher: Arc<dyn VoxelMesher> = Arc::new(VoxelDataLockProbeMesher {
            data: Arc::downgrade(&data),
            build_calls: build_calls.clone(),
        });
        let generator: Arc<dyn VoxelGenerator> = Arc::new(ConstantSdfGenerator { value: -1.0 });
        let meshing_dep = MeshingDependency::new(mesher, Some(generator));

        let mut task = MeshBlockTask::new(MeshBlockTaskParams {
            position_in_blocks: Vector3i::zero(),
            lod_index: 0,
            data,
            meshing_dependency: meshing_dep,
            collision_hint: false,
            lod_hint: false,
        });

        task.run_meshing();
        let output = task.take_output().expect("task produced output");

        assert!(!output.dropped);
        assert_eq!(*build_calls.lock().unwrap(), 1);
    }

    #[test]
    fn mesh_block_tasks_can_overlap_inside_shared_mesher() {
        let data = shared_data_with_central_block(16);
        let entered = Arc::new((Mutex::new(0), Condvar::new()));
        let inside = Arc::new(AtomicUsize::new(0));
        let max_inside = Arc::new(AtomicUsize::new(0));
        let mesher: Arc<dyn VoxelMesher> = Arc::new(OverlapProbeMesher {
            entered,
            inside,
            max_inside: max_inside.clone(),
        });
        let meshing_dep = MeshingDependency::new(mesher, None);

        let make_task = |position_in_blocks| {
            MeshBlockTask::new(MeshBlockTaskParams {
                position_in_blocks,
                lod_index: 0,
                data: data.clone(),
                meshing_dependency: meshing_dep.clone(),
                collision_hint: false,
                lod_hint: false,
            })
        };
        let mut first = make_task(Vector3i::zero());
        let mut second = make_task(Vector3i::new(1, 0, 0));

        let first = std::thread::spawn(move || {
            first.run_meshing();
            first.take_output().expect("first task produced output")
        });
        let second = std::thread::spawn(move || {
            second.run_meshing();
            second.take_output().expect("second task produced output")
        });

        let first = first.join().expect("first mesh task completed");
        let second = second.join().expect("second mesh task completed");

        assert!(!first.dropped);
        assert!(!second.dropped);
        assert_eq!(max_inside.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn mesh_block_task_emits_dropped_output_when_dependency_invalidated() {
        let data = shared_data_with_central_block(16);
        let build_calls = Arc::new(Mutex::new(0));
        let mesher: Arc<dyn VoxelMesher> = Arc::new(DummyMesher {
            build_calls: build_calls.clone(),
        });
        let meshing_dep = MeshingDependency::new(mesher, None);

        let mut task = MeshBlockTask::new(MeshBlockTaskParams {
            position_in_blocks: Vector3i::zero(),
            lod_index: 0,
            data: data.clone(),
            meshing_dependency: meshing_dep.clone(),
            collision_hint: false,
            lod_hint: false,
        });

        // Invalidate the dependency before running; the task must not call
        // the mesher and must emit a dropped output.
        meshing_dep.invalidate();
        task.run_meshing();
        let output = task.take_output().expect("task produced output");

        assert!(output.dropped);
        assert!(output.surfaces.is_empty());
        assert_eq!(*build_calls.lock().unwrap(), 0);
    }

    #[test]
    fn mesh_block_task_implements_threaded_task_contract() {
        let data = shared_data_with_central_block(16);
        let mesher: Arc<dyn VoxelMesher> = Arc::new(DummyMesher {
            build_calls: Arc::new(Mutex::new(0)),
        });
        let meshing_dep = MeshingDependency::new(mesher, None);

        let mut task = MeshBlockTask::new(MeshBlockTaskParams {
            position_in_blocks: Vector3i::new(3, 4, 5),
            lod_index: 2,
            data,
            meshing_dependency: meshing_dep,
            collision_hint: true,
            lod_hint: true,
        });

        use crate::tasks::{ThreadedTask, ThreadedTaskContext};
        assert_eq!(task.position_in_blocks(), Vector3i::new(3, 4, 5));
        assert_eq!(task.lod_index(), 2);
        assert_eq!(task.debug_name(), "MeshBlockTask");
        assert!(!task.is_cancelled());

        // Run via the trait method (the threaded-task entry point) and then
        // recover the concrete task to inspect its output.
        let outcome = Box::new(task).run(ThreadedTaskContext::new(
            0,
            crate::tasks::TaskPriority::max(),
        ));
        match outcome {
            crate::tasks::TaskRunOutcome::Complete(completed) => {
                // Recover the concrete type via Any-downcast not available on
                // the trait object; instead we re-run on a fresh task to
                // inspect the output struct directly.
                drop(completed);
            }
            _ => panic!("MeshBlockTask must complete"),
        }

        // Separate run via run_meshing to assert the output struct shape.
        let mut fresh = MeshBlockTask::new(MeshBlockTaskParams {
            position_in_blocks: Vector3i::new(3, 4, 5),
            lod_index: 2,
            data: shared_data_with_central_block(16),
            meshing_dependency: MeshingDependency::new(
                Arc::new(DummyMesher {
                    build_calls: Arc::new(Mutex::new(0)),
                }),
                None,
            ),
            collision_hint: true,
            lod_hint: true,
        });
        fresh.run_meshing();
        let output = fresh.take_output().unwrap();
        assert!(!output.dropped);
    }
}
