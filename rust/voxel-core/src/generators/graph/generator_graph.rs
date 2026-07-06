//! [`GraphGenerator`] — adapts a [`Graph`] to the [`VoxelGenerator`] trait.
//!
//! Ports the engine-agnostic half of `generators/graph/voxel_generator_graph.cpp`
//! — specifically the `generate_block` loop that walks a `VoxelBuffer` in
//! Y-slices, runs the runtime over each slice, and copies the SDF output back
//! into the SDF channel. Skips the Godot `Resource`/editor/GPU/serialization
//! machinery; that lives in `voxel-gdext`.

use crate::generators::base::{GenResult, VoxelGenerator, VoxelQueryData};
use crate::generators::graph::{
    Graph, GraphInputs, GraphNodeId, GraphOutput, GraphScratch, NodeKind,
};
use crate::math::Vector3i;
use crate::storage::voxel_buffer::ChannelId;
use crate::storage::VoxelBuffer;
use std::sync::Mutex;

/// Wraps a [`Graph`] in a [`VoxelGenerator`] that fills a `VoxelBuffer` block
/// by executing the graph one Y-slice at a time. The graph must contain at
/// least one `OutputSdf` node; otherwise `generate_block` is a no-op.
pub struct GraphGenerator {
    graph: Graph,
    /// Per-instance scratch. The generator trait is shared (`&self`) so the
    /// scratch owns its synchronization locally instead of forcing every
    /// generator call through an outer engine-wide mutex.
    scratch: Mutex<GraphScratch>,
    /// Optional scaling applied to world coordinates before they're fed into
    /// the graph (mirrors C++ `lod` stride handling). `1.0` is the identity.
    coordinate_scale: f32,
}

impl GraphGenerator {
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            scratch: Mutex::new(GraphScratch::new()),
            coordinate_scale: 1.0,
        }
    }

    /// Scales input coordinates by `scale` (useful for LOD: `1 << lod`).
    pub fn with_coordinate_scale(mut self, scale: f32) -> Self {
        self.coordinate_scale = scale;
        self
    }

    /// Read-only access to the underlying graph.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Returns the node id of the first `OutputSdf` node, if any. Used by
    /// tests to assert the graph has at least one output before generation.
    pub fn first_sdf_output(&self) -> Option<GraphNodeId> {
        self.graph
            .nodes()
            .iter()
            .find(|n| matches!(n.kind, NodeKind::OutputSdf { .. }))
            .map(|n| n.id)
    }
}

impl VoxelGenerator for GraphGenerator {
    fn generate_block(&self, input: VoxelQueryData<'_>) -> GenResult {
        let mut scratch = self
            .scratch
            .lock()
            .expect("graph generator scratch poisoned");
        generate_block_with_graph(&self.graph, input, &mut scratch, self.coordinate_scale);
        GenResult::default()
    }

    fn used_channels_mask(&self) -> u32 {
        // The minimal port always writes the SDF channel.
        1 << ChannelId::Sdf.index()
    }
}

/// Free-function form of [`GraphGenerator::generate_block`], exposed so a
/// caller can drive a shared `&Graph` (e.g. behind an `Arc<Mutex<>>`) without
/// going through the trait. The C++ side has the same split (the runtime is
/// independent of the `VoxelGenerator` wrapper).
pub fn generate_block_with_graph(
    graph: &Graph,
    input: VoxelQueryData<'_>,
    scratch: &mut GraphScratch,
    coordinate_scale: f32,
) {
    let size = input.buffer.size();
    let sdf_channel = ChannelId::Sdf.index();
    let lod = input.lod;
    let lod_stride = (1u32 << lod) as f32;

    // Pre-allocated scratch buffers for the per-slice X/Z world coordinates.
    // Reused across Y-slices to avoid reallocation. The slice has
    // `size.x * size.z` voxels (ZXY layout — Y innermost).
    let slice_size = (size.x as usize) * (size.z as usize);
    let mut xs: Vec<f32> = vec![0.0; slice_size];
    let mut zs: Vec<f32> = vec![0.0; slice_size];
    let mut outputs: Vec<(GraphOutput, Vec<f32>)> = Vec::new();

    for y in 0..size.y {
        let world_y = (input.origin_in_voxels.y as f32 + y as f32 * lod_stride) * coordinate_scale;
        // Build the X and Z slices. ZXY layout: for each z, for each x, the
        // voxel index is `y + size.y * (x + size.x * z)` — but we only need
        // the world-space (x, z) per slice cell, which is independent of y.
        for z in 0..size.z {
            for x in 0..size.x {
                let i = (x as usize) + (z as usize) * (size.x as usize);
                xs[i] =
                    (input.origin_in_voxels.x as f32 + x as f32 * lod_stride) * coordinate_scale;
                zs[i] =
                    (input.origin_in_voxels.z as f32 + z as f32 * lod_stride) * coordinate_scale;
            }
        }

        let inputs = GraphInputs {
            x: &xs,
            y: world_y,
            z: &zs,
        };
        if graph
            .generate(&inputs, slice_size, scratch, &mut outputs)
            .is_err()
        {
            // Topology error: bail out (matches the C++ behaviour of
            // printing an error and leaving the block at its default).
            return;
        }

        // Copy the first SDF output (if any) into the VoxelBuffer's SDF
        // channel for this slice. The C++ runtime supports multiple outputs;
        // the minimal port merges them by writing only the first.
        if let Some((GraphOutput::Sdf, slice)) = outputs.first() {
            write_sdf_slice(input.buffer, sdf_channel, size, y, slice);
        }
    }
}

fn write_sdf_slice(
    buffer: &mut VoxelBuffer,
    channel: usize,
    size: Vector3i,
    y: i32,
    slice: &[f32],
) {
    for z in 0..size.z {
        for x in 0..size.x {
            let i = (x as usize) + (z as usize) * (size.x as usize);
            buffer.set_voxel_f(slice[i], x, y, z, channel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::graph::{GraphPort, NodeKind};
    use crate::math::Vector3i;
    use crate::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat};

    /// Build a graph that computes `sin(x) + 1` and writes the result to the
    /// SDF channel. With a constant offset, every voxel of the resulting
    /// buffer is `>= 1.0`, so the block is fully "outside" any iso-surface.
    fn sin_plus_one_graph() -> Graph {
        let mut graph = Graph::new();
        let x = graph.push(NodeKind::InputX);
        let sin = graph.push(NodeKind::Sin {
            a: Some(GraphPort::new(x)),
        });
        let one = graph.push(NodeKind::Constant(1.0));
        let add = graph.push(NodeKind::Add {
            a: Some(GraphPort::new(sin)),
            b: Some(GraphPort::new(one)),
        });
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(add)),
        });
        graph
    }

    #[test]
    fn generate_block_writes_sin_plus_one_into_sdf_channel() {
        let graph = sin_plus_one_graph();
        let generator = GraphGenerator::new(graph);

        let mut buffer = VoxelBuffer::with_size(Vector3i::new(4, 2, 4));
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        format.configure_buffer(&mut buffer);

        let origin = Vector3i::new(10, 0, 0);
        let _ = generator.generate_block(VoxelQueryData {
            buffer: &mut buffer,
            origin_in_voxels: origin,
            lod: 0,
        });

        // SDF value at (x=10, y=0, z=0) should be sin(10) + 1.
        let expected = (10.0f32).sin() + 1.0;
        let actual = buffer.get_voxel_f(0, 0, 0, ChannelId::Sdf.index());
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}"
        );

        // And at (x=12, y=1, z=2).
        let expected2 = (12.0f32).sin() + 1.0;
        let actual2 = buffer.get_voxel_f(2, 1, 2, ChannelId::Sdf.index());
        assert!(
            (actual2 - expected2).abs() < 1e-4,
            "expected {expected2}, got {actual2}"
        );
    }

    #[test]
    fn generate_block_skips_silently_when_the_graph_has_no_output() {
        let mut graph = Graph::new();
        let _ = graph.push(NodeKind::InputX); // no OutputSdf
        let generator = GraphGenerator::new(graph);

        let mut buffer = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        format.configure_buffer(&mut buffer);

        let _ = generator.generate_block(VoxelQueryData {
            buffer: &mut buffer,
            origin_in_voxels: Vector3i::zero(),
            lod: 0,
        });

        // Buffer stays at the SDF default (SDF_FAR_OUTSIDE).
        assert_eq!(
            buffer.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()),
            crate::storage::voxel_buffer::SDF_FAR_OUTSIDE
        );
    }

    #[test]
    fn coordinate_scale_stretches_input_coordinates() {
        let mut graph = Graph::new();
        let x = graph.push(NodeKind::InputX);
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(x)),
        });
        let generator = GraphGenerator::new(graph).with_coordinate_scale(2.0);

        let mut buffer = VoxelBuffer::with_size(Vector3i::new(2, 1, 1));
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        format.configure_buffer(&mut buffer);

        let _ = generator.generate_block(VoxelQueryData {
            buffer: &mut buffer,
            origin_in_voxels: Vector3i::new(10, 0, 0),
            lod: 0,
        });

        // X is scaled by 2.0: voxel (0,0,0) gets world X 10*2 = 20.
        assert_eq!(buffer.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()), 20.0);
        // Voxel (1,0,0) gets world X 11*2 = 22.
        assert_eq!(buffer.get_voxel_f(1, 0, 0, ChannelId::Sdf.index()), 22.0);
    }

    #[test]
    fn lod_stride_scales_local_coordinates_without_scaling_origin() {
        let mut graph = Graph::new();
        let x = graph.push(NodeKind::InputX);
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(x)),
        });
        let generator = GraphGenerator::new(graph);

        let mut buffer = VoxelBuffer::with_size(Vector3i::new(2, 1, 1));
        let mut format = VoxelFormat::new();
        format.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        format.configure_buffer(&mut buffer);

        let _ = generator.generate_block(VoxelQueryData {
            buffer: &mut buffer,
            origin_in_voxels: Vector3i::new(10, 0, 0),
            lod: 1,
        });

        assert_eq!(buffer.get_voxel_f(0, 0, 0, ChannelId::Sdf.index()), 10.0);
        assert_eq!(buffer.get_voxel_f(1, 0, 0, ChannelId::Sdf.index()), 12.0);
    }

    #[test]
    fn used_channels_mask_targets_sdf_only() {
        let graph = Graph::new();
        let generator = GraphGenerator::new(graph);
        assert_eq!(
            generator.used_channels_mask(),
            1u32 << ChannelId::Sdf.index()
        );
    }

    #[test]
    fn first_sdf_output_finds_the_output_node() {
        let graph = sin_plus_one_graph();
        let generator = GraphGenerator::new(graph);
        assert!(generator.first_sdf_output().is_some());

        let mut empty = Graph::new();
        empty.push(NodeKind::InputX);
        let empty_gen = GraphGenerator::new(empty);
        assert!(empty_gen.first_sdf_output().is_none());
    }

    /// `Send + Sync` is required by `VoxelGenerator` so the graph generator
    /// can live behind `Arc<dyn VoxelGenerator>`.
    #[test]
    fn graph_generator_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GraphGenerator>();
    }
}
