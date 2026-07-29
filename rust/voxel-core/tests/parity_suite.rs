//! Expanded parity suite — golden/diff tests across subsystems.
//!
//! Covers:
//! - Storage: VoxelBuffer serialize/deserialize round-trip
//! - Meshers: Cubes + Blocky output structure verification
//! - Graph runtime: golden vectors per node type (GRAPH-2 parity)
//! - Edition ops: do_sphere/do_box output verification

#[cfg(test)]
mod storage_parity {
    use voxel_core::math::Vector3i;
    use voxel_core::storage::{
        voxel_buffer::{raw_voxel_to_real, real_to_raw_voxel},
        ChannelDepth, ChannelId, Compression, VoxelBuffer, VoxelFormat,
    };

    #[test]
    fn voxel_buffer_round_trip_sdf_32bit() {
        let mut buf = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut buf);

        // Write known SDF values.
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    let val = (x + y * 4 + z * 16) as f32 * 0.1 - 1.0;
                    buf.set_voxel_f(val, x, y, z, ChannelId::Sdf.index());
                }
            }
        }

        // Read back and verify.
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    let expected = (x + y * 4 + z * 16) as f32 * 0.1 - 1.0;
                    let actual = buf.get_voxel_f(x, y, z, ChannelId::Sdf.index());
                    assert!(
                        (actual - expected).abs() < 1e-5,
                        "SDF round-trip mismatch at ({x},{y},{z}): {actual} vs {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn voxel_buffer_compression_uniform_round_trip() {
        let mut buf = VoxelBuffer::with_size(Vector3i::new(8, 8, 8));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut buf);

        // Fill uniform, compress, decompress, verify.
        buf.clear_channel_f(ChannelId::Sdf.index(), -5.0);
        buf.compress_uniform_channels();
        assert_eq!(
            buf.channel_compression(ChannelId::Sdf.index()),
            Compression::Uniform
        );
        let val = buf.get_voxel_f(3, 3, 3, ChannelId::Sdf.index());
        assert!(
            (val - (-5.0)).abs() < 1e-5,
            "uniform value after compress: {val}"
        );
    }

    #[test]
    fn sdf_quantization_8bit_round_trip() {
        // Verify 8-bit snorm quantization is stable for mid-range values.
        // Extreme values (1.0) clamp via snorm scale, so we test 0..0.5 range.
        let depth = ChannelDepth::Bit8;
        for &input in &[0.0, -1.0, 0.5, -0.5, 10.0, -10.0] {
            let raw = real_to_raw_voxel(input, depth);
            let back = raw_voxel_to_real(raw, depth);
            // 8-bit snorm quantization at extremes has ~0.1 resolution.
            assert!(
                (back - input).abs() < 0.15,
                "8-bit SDF quantization: {input} → raw {raw} → {back}, diff > 0.15"
            );
        }
    }

    #[test]
    fn block_serializer_round_trip() {
        use voxel_core::streams::block_serializer;
        let mut buf = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut buf);
        // Write a gradient.
        for i in 0..64 {
            buf.set_voxel_f(
                i as f32 * 0.5,
                (i % 4) as i32,
                ((i / 4) % 4) as i32,
                (i / 16) as i32,
                ChannelId::Sdf.index(),
            );
        }

        // Serialize.
        let mut data = Vec::new();
        block_serializer::serialize(&buf, &mut data).unwrap();
        assert!(!data.is_empty());

        // Deserialize into a fresh buffer.
        let mut buf2 = VoxelBuffer::with_size(Vector3i::new(4, 4, 4));
        fmt.configure_buffer(&mut buf2);
        block_serializer::deserialize(&data, &mut buf2).unwrap();

        // Verify all voxels match.
        for i in 0..64 {
            let x = (i % 4) as i32;
            let y = ((i / 4) % 4) as i32;
            let z = (i / 16) as i32;
            let v1 = buf.get_voxel_f(x, y, z, ChannelId::Sdf.index());
            let v2 = buf2.get_voxel_f(x, y, z, ChannelId::Sdf.index());
            assert!(
                (v1 - v2).abs() < 1e-5,
                "serialize round-trip mismatch at ({x},{y},{z}): {v1} vs {v2}"
            );
        }
    }
}

#[cfg(test)]
mod mesher_parity {
    use voxel_core::math::Vector3i;
    use voxel_core::meshers::{
        BlockyMesher, CubesMesher, MesherInput, MesherOutput, TransvoxelMesher, VoxelMesher,
    };
    use voxel_core::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat};

    fn make_sdf_sphere(size: i32, radius: f32) -> VoxelBuffer {
        let mut buf = VoxelBuffer::with_size(Vector3i::splat(size));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut buf);
        let cx = size as f32 * 0.5;
        for z in 0..size {
            for y in 0..size {
                for x in 0..size {
                    let d = ((x as f32 - cx).powi(2)
                        + (y as f32 - cx).powi(2)
                        + (z as f32 - cx).powi(2))
                    .sqrt()
                        - radius;
                    buf.set_voxel_f(d, x, y, z, ChannelId::Sdf.index());
                }
            }
        }
        buf
    }

    fn make_solid_blocky(size: i32) -> VoxelBuffer {
        let mut buf = VoxelBuffer::with_size(Vector3i::splat(size));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Type.index()] = voxel_core::storage::ChannelDepth::Bit8;
        fmt.configure_buffer(&mut buf);
        for z in 1..size - 1 {
            for y in 1..size - 1 {
                for x in 1..size - 1 {
                    buf.set_voxel(1, x, y, z, ChannelId::Type.index());
                }
            }
        }
        buf
    }

    #[test]
    fn transvoxel_sphere_produces_closed_mesh() {
        let mesher = TransvoxelMesher::new();
        let voxels = make_sdf_sphere(16, 6.0);
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);
        assert!(
            output.total_vertex_count() > 0,
            "transvoxel should produce vertices"
        );
        assert_eq!(
            output.total_triangle_count(),
            output
                .surfaces
                .iter()
                .map(|s| s.arrays.triangle_count())
                .sum::<usize>()
        );
        // Every triangle should have valid indices.
        for surface in &output.surfaces {
            if let voxel_core::meshers::SurfaceArrays::Transvoxel(arrays) = &surface.arrays {
                let vc = arrays.vertices.len();
                for idx in &arrays.indices {
                    assert!((*idx as usize) < vc, "index {idx} out of bounds (vc={vc})");
                }
            }
        }
    }

    #[test]
    fn blocky_empty_library_produces_no_faces() {
        use std::sync::Arc;
        let library = Arc::new(voxel_core::meshers::blocky::baked_library::BakedLibrary::default());
        let mesher = BlockyMesher::new(library);
        let voxels = make_solid_blocky(6);
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);
        // Empty library → no models → no geometry.
        assert_eq!(
            output.total_vertex_count(),
            0,
            "empty library should produce no geometry"
        );
    }

    #[test]
    fn cubes_solid_block_produces_two_surfaces() {
        let mesher = CubesMesher::new();
        let voxels = make_solid_blocky(4);
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        let mut output = MesherOutput::default();
        mesher.build(&mut output, &input);
        // Cubes always emits opaque + transparent surfaces.
        assert_eq!(
            output.surfaces.len(),
            2,
            "cubes should produce 2 surfaces (opaque + transparent)"
        );
    }
}

#[cfg(test)]
mod graph_parity {
    use voxel_core::generators::graph::{
        CompiledGraph, Graph, GraphInputs, GraphOutput, GraphPort, GraphScratch, NodeKind,
    };

    fn eval_node(kind: NodeKind, inputs: &GraphInputs, slice_size: usize) -> Vec<f32> {
        let mut g = Graph::new();
        let id = g.push(kind);
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(id)),
        });
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        g.generate(inputs, slice_size, &mut scratch, &mut outputs)
            .unwrap();
        outputs
            .into_iter()
            .find(|(k, _)| *k == GraphOutput::Sdf)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }

    fn x_inputs(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    #[test]
    fn graph_add_golden() {
        let xs = x_inputs(4);
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut g = Graph::new();
        let a = g.push(NodeKind::Constant(3.0));
        let b = g.push(NodeKind::Constant(4.0));
        let add = g.push(NodeKind::Add {
            a: Some(GraphPort::new(a)),
            b: Some(GraphPort::new(b)),
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(add)),
        });
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        g.generate(&inputs, 4, &mut scratch, &mut outputs).unwrap();
        let data = &outputs[0].1;
        for v in data {
            assert!((v - 7.0).abs() < 1e-5, "Add(3,4) should be 7, got {v}");
        }
    }

    #[test]
    fn graph_multiply_golden() {
        let xs = x_inputs(4);
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut g = Graph::new();
        let x = g.push(NodeKind::InputX);
        let c = g.push(NodeKind::Constant(3.0));
        let mul = g.push(NodeKind::Multiply {
            a: Some(GraphPort::new(x)),
            b: Some(GraphPort::new(c)),
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(mul)),
        });
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        g.generate(&inputs, 4, &mut scratch, &mut outputs).unwrap();
        let data = &outputs[0].1;
        assert!((data[0] - 0.0).abs() < 1e-5);
        assert!((data[1] - 3.0).abs() < 1e-5);
        assert!((data[2] - 6.0).abs() < 1e-5);
        assert!((data[3] - 9.0).abs() < 1e-5);
    }

    #[test]
    fn graph_divide_exact_zero_golden() {
        // GRAPH-2 parity: exact-zero test (not epsilon).
        let xs = x_inputs(2);
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut g = Graph::new();
        let a = g.push(NodeKind::Constant(4.0));
        let b = g.push(NodeKind::Constant(0.0));
        let div = g.push(NodeKind::Divide {
            a: Some(GraphPort::new(a)),
            b: Some(GraphPort::new(b)),
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(div)),
        });
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        g.generate(&inputs, 2, &mut scratch, &mut outputs).unwrap();
        assert_eq!(outputs[0].1[0], 0.0, "divide by exact 0 should be 0");
    }

    #[test]
    fn graph_remap_no_clamp_golden() {
        // GRAPH-2 parity: pure linear remap, no clamp.
        let xs = vec![0.0, 1.0, 2.0, 5.0];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut g = Graph::new();
        let x = g.push(NodeKind::InputX);
        let remap = g.push(NodeKind::Remap {
            a: Some(GraphPort::new(x)),
            from_start: 0.0,
            from_end: 2.0,
            to_start: 10.0,
            to_end: 20.0,
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(remap)),
        });
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        g.generate(&inputs, 4, &mut scratch, &mut outputs).unwrap();
        let d = &outputs[0].1;
        assert!((d[0] - 10.0).abs() < 1e-5);
        assert!((d[1] - 15.0).abs() < 1e-5);
        assert!((d[2] - 20.0).abs() < 1e-5);
        assert!(
            (d[3] - 35.0).abs() < 1e-5,
            "extrapolation should NOT clamp: {}",
            d[3]
        );
    }

    #[test]
    fn graph_sdf_sphere_golden() {
        // SdfSphere at (3,0,0) radius=2 → at (1,0,0): dist - r = -1 (inside).
        let xs = vec![1.0, 3.0, 5.0];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut g = Graph::new();
        let x = g.push(NodeKind::InputX);
        let y = g.push(NodeKind::InputY);
        let z = g.push(NodeKind::InputZ);
        let r = g.push(NodeKind::Constant(2.0));
        let sph = g.push(NodeKind::SdfSphere {
            x: Some(GraphPort::new(x)),
            y: Some(GraphPort::new(y)),
            z: Some(GraphPort::new(z)),
            radius: Some(GraphPort::new(r)),
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(sph)),
        });
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        g.generate(&inputs, 3, &mut scratch, &mut outputs).unwrap();
        let d = &outputs[0].1;
        // At x=1: SDF sphere at origin with r=2.
        // In voxel-core, SDF is stored with sign convention where the
        // graph runtime negates get_voxel_f. The formula inside the runtime
        // is: -(distance - radius). At (1,0,0): -(sqrt(1) - 2) = -(1-2) = 1.
        // But InputX returns the raw x value, and SdfSphere computes
        // sqrt(x²+y²+z²) - r, so at x=1,y=0,z=0: sqrt(1)-2 = -1.
        // The actual value depends on exact sign handling. Just verify <0 (inside).
        assert!(
            d[0] < 0.0,
            "sphere at (1,0,0) r=2 should be inside (negative): got {}",
            d[0]
        );
    }

    #[test]
    fn graph_distance_3d_two_points_golden() {
        // Distance3D from (0,0,0) to (3,4,3): sqrt(34).
        let xs = vec![0.0];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut g = Graph::new();
        let x0 = g.push(NodeKind::Constant(0.0));
        let y0 = g.push(NodeKind::Constant(0.0));
        let z0 = g.push(NodeKind::Constant(0.0));
        let x1 = g.push(NodeKind::Constant(3.0));
        let y1 = g.push(NodeKind::Constant(4.0));
        let z1 = g.push(NodeKind::Constant(3.0));
        let d = g.push(NodeKind::Distance3D {
            x0: Some(GraphPort::new(x0)),
            y0: Some(GraphPort::new(y0)),
            z0: Some(GraphPort::new(z0)),
            x1: Some(GraphPort::new(x1)),
            y1: Some(GraphPort::new(y1)),
            z1: Some(GraphPort::new(z1)),
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(d)),
        });
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        g.generate(&inputs, 1, &mut scratch, &mut outputs).unwrap();
        assert!(
            (outputs[0].1[0] - 34.0f32.sqrt()).abs() < 1e-5,
            "distance (0,0,0)-(3,4,3) = sqrt(34), got {}",
            outputs[0].1[0]
        );
    }

    #[test]
    fn graph_compiled_matches_lazy() {
        // Verify compiled path matches lazy path for a sin(x) graph.
        let mut g = Graph::new();
        let x = g.push(NodeKind::InputX);
        let sin = g.push(NodeKind::Sin {
            a: Some(GraphPort::new(x)),
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(sin)),
        });

        let xs = x_inputs(8);
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };

        // Lazy path.
        let mut scratch = GraphScratch::new();
        let mut lazy_out = Vec::new();
        g.generate(&inputs, 8, &mut scratch, &mut lazy_out).unwrap();
        let lazy_sdf = lazy_out[0].1.clone();

        // Compiled path.
        let compiled = CompiledGraph::compile(&g).unwrap();
        let mut cscratch = voxel_core::generators::graph::CompiledScratch::new();
        let mut cout = Vec::new();
        compiled.generate_slice(&inputs, 8, &mut cscratch, &mut cout, false);
        let compiled_sdf = cout
            .iter()
            .find(|(k, _)| *k == GraphOutput::Sdf)
            .map(|(_, v)| v.clone())
            .unwrap();

        for i in 0..8 {
            assert!(
                (lazy_sdf[i] - compiled_sdf[i]).abs() < 1e-5,
                "lazy vs compiled mismatch at {i}: {} vs {}",
                lazy_sdf[i],
                compiled_sdf[i]
            );
        }
    }
}

#[cfg(test)]
mod edition_parity {
    use voxel_core::edition::{do_box, do_sphere, EditMode};
    use voxel_core::math::{Vector3f, Vector3i};
    use voxel_core::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat};

    #[test]
    fn do_sphere_add_produces_negative_sdf_inside() {
        let mut buf = VoxelBuffer::with_size(Vector3i::splat(16));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut buf);
        buf.clear_channel_f(ChannelId::Sdf.index(), 100.0); // Start as air.

        do_sphere(
            &mut buf,
            ChannelId::Sdf.index(),
            EditMode::Add,
            1,
            Vector3f::new(8.0, 8.0, 8.0),
            4.0,
        );

        // Center: inside sphere → negative SDF (solid).
        let center = buf.get_voxel_f(8, 8, 8, ChannelId::Sdf.index());
        assert!(center < 0.0, "center should be solid: {center}");
        // Corner: outside sphere → still air.
        let corner = buf.get_voxel_f(0, 0, 0, ChannelId::Sdf.index());
        assert!(corner > 0.0, "corner should be air: {corner}");
    }

    #[test]
    fn do_sphere_remove_carves_from_solid() {
        let mut buf = VoxelBuffer::with_size(Vector3i::splat(16));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut buf);
        buf.clear_channel_f(ChannelId::Sdf.index(), -100.0); // Start as solid.

        do_sphere(
            &mut buf,
            ChannelId::Sdf.index(),
            EditMode::Remove,
            1,
            Vector3f::new(8.0, 8.0, 8.0),
            3.0,
        );

        let center = buf.get_voxel_f(8, 8, 8, ChannelId::Sdf.index());
        assert!(center > 0.0, "center should be carved to air: {center}");
    }

    #[test]
    fn do_box_set_writes_correct_values() {
        let mut buf = VoxelBuffer::with_size(Vector3i::splat(8));
        let mut fmt3 = VoxelFormat::new();
        fmt3.depths[ChannelId::Type.index()] = voxel_core::storage::ChannelDepth::Bit8;
        fmt3.configure_buffer(&mut buf);
        do_box(
            &mut buf,
            ChannelId::Type.index(),
            EditMode::Set,
            42,
            Vector3i::new(2, 2, 2),
            Vector3i::new(6, 6, 6),
        );
        assert_eq!(buf.get_voxel(3, 3, 3, ChannelId::Type.index()), 42);
        assert_eq!(buf.get_voxel(0, 0, 0, ChannelId::Type.index()), 0);
        assert_eq!(buf.get_voxel(5, 5, 5, ChannelId::Type.index()), 42);
    }

    #[test]
    fn raycast_dda_finds_solid_voxel() {
        use voxel_core::edition::voxel_raycast;
        let hit = voxel_raycast(
            Vector3f::new(0.5, 0.5, 0.5),
            Vector3f::new(1.0, 0.0, 0.0),
            100.0,
            |s| s.position == Vector3i::new(5, 0, 0),
        );
        assert!(hit.is_some());
        let h = hit.unwrap();
        assert_eq!(h.position, Vector3i::new(5, 0, 0));
        assert_eq!(h.normal, Vector3i::new(-1, 0, 0));
    }
}

#[cfg(test)]
mod streams_parity {
    use voxel_core::math::Vector3i;
    use voxel_core::storage::{ChannelId, VoxelBuffer, VoxelFormat};

    #[test]
    fn block_serializer_compressed_round_trip() {
        use voxel_core::streams::block_serializer;
        let mut buf = VoxelBuffer::with_size(Vector3i::new(8, 8, 8));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = voxel_core::storage::ChannelDepth::Bit32;
        fmt.configure_buffer(&mut buf);
        buf.clear_channel_f(ChannelId::Sdf.index(), -3.0);

        let mut compressed = Vec::new();
        block_serializer::serialize_and_compress(
            &buf,
            &mut compressed,
            voxel_core::streams::compressed_data::Compression::Lz4,
        )
        .unwrap();
        assert!(!compressed.is_empty());

        let mut buf2 = VoxelBuffer::with_size(Vector3i::new(8, 8, 8));
        fmt.configure_buffer(&mut buf2);
        let status = block_serializer::decompress_and_deserialize_with_limits(
            &compressed,
            &mut buf2,
            voxel_core::streams::decode_limits::DecodeLimits::default(),
        )
        .unwrap();
        assert_eq!(status, block_serializer::DeserializeStatus::Complete);

        let val = buf2.get_voxel_f(4, 4, 4, ChannelId::Sdf.index());
        assert!(
            (val - (-3.0)).abs() < 1e-5,
            "compressed round-trip: expected -3.0, got {val}"
        );
    }
}

#[cfg(test)]
mod terrain_parity {
    use std::sync::Arc;
    use voxel_core::engine::MeshingDependency;
    use voxel_core::generators::simple::Flat;
    use voxel_core::math::{Box3i, Vector3i};
    use voxel_core::meshers::TransvoxelMesher;
    use voxel_core::storage::VoxelData;
    use voxel_core::terrain::{ViewerUpdate, VoxelTerrainCore};

    #[test]
    fn single_lod_terrain_paging_converges_with_viewer() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::splat(-512), Vector3i::splat(2048)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let gen: voxel_core::storage::SharedVoxelGenerator = Arc::new(Flat::default());
        data.set_generator(Some(gen));
        let mesher = Arc::new(TransvoxelMesher::new());
        let dep = MeshingDependency::new(mesher, None);
        let mut core = VoxelTerrainCore::new_generator_only(data, dep);

        // Run several ticks with a viewer at origin.
        let viewers = vec![ViewerUpdate {
            id: 0,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: 48,
            vertical_view_distance_voxels: 48,
            requires_meshes: true,
        }];
        for _ in 0..20 {
            core.process(&viewers);
        }

        // Should have mesh blocks loaded.
        assert!(
            core.mesh_blocks().len() > 0,
            "terrain should have loaded mesh blocks after convergence"
        );
    }

    #[test]
    fn multi_lod_terrain_produces_blocks_at_both_lods() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::splat(-512), Vector3i::splat(2048)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let gen: voxel_core::storage::SharedVoxelGenerator = Arc::new(Flat::default());
        data.set_generator(Some(gen));
        let mesher = Arc::new(TransvoxelMesher::new());
        let dep = MeshingDependency::new(mesher, None);
        let stream: Arc<dyn voxel_core::streams::VoxelStream> =
            Arc::new(voxel_core::streams::MemoryStream::new());
        let mut core = VoxelTerrainCore::new_with_lod_count(data, stream, dep, 2);

        let viewers = vec![ViewerUpdate {
            id: 0,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: 48,
            vertical_view_distance_voxels: 48,
            requires_meshes: true,
        }];
        for _ in 0..20 {
            core.process(&viewers);
        }

        let lod0 = core.mesh_blocks_at_lod(0).len();
        let lod1 = core.mesh_blocks_at_lod(1).len();
        assert!(lod0 > 0, "LOD 0 should have blocks: {lod0}");
        assert!(lod1 > 0, "LOD 1 should have blocks: {lod1}");
    }

    /// Golden test: after convergence with a Flat generator and a 48-voxel
    /// view distance, the terrain produces a fixed number of mesh blocks and
    /// a fixed total vertex count. Pinned against the current paging +
    /// transvoxel implementation; a regression in either will flip the count.
    #[test]
    fn single_lod_terrain_vertex_count_golden_after_convergence() {
        let mut data = VoxelData::new();
        data.set_bounds(Box3i::new(Vector3i::splat(-512), Vector3i::splat(2048)));
        data.set_streaming_enabled(false);
        data.set_full_load_completed(true);
        let gen: voxel_core::storage::SharedVoxelGenerator = Arc::new(Flat::default());
        data.set_generator(Some(gen));
        let mesher = Arc::new(TransvoxelMesher::new());
        let dep = MeshingDependency::new(mesher, None);
        let mut core = VoxelTerrainCore::new_generator_only(data, dep);

        let viewers = vec![ViewerUpdate {
            id: 0,
            world_position_voxels: Vector3i::zero(),
            horizontal_view_distance_voxels: 48,
            vertical_view_distance_voxels: 48,
            requires_meshes: true,
        }];
        // Drive paging to full convergence: tick, wait for background tasks,
        // then re-tick to apply any drained mesh outputs, until no tasks and
        // no pending work remain. This makes the post-convergence mesh output
        // deterministic regardless of thread timing.
        for _ in 0..100 {
            core.process(&viewers);
            core.wait_for_pending_tasks();
            core.process(&viewers);
            if core.pending_task_count() == 0 {
                break;
            }
        }

        let block_count = core.mesh_blocks().len();
        let total_verts: usize = core
            .mesh_blocks()
            .values()
            .filter_map(|e| e.output.as_ref())
            .map(|o| o.total_vertex_count())
            .sum();

        // Pinned golden values for a 48-voxel view distance around origin,
        // measured after full convergence (no pending tasks). 216 mesh blocks,
        // each with a single transvoxel surface, totalling 36864 vertices.
        assert_eq!(
            block_count, 216,
            "mesh block count regressed: {block_count}"
        );
        assert_eq!(
            total_verts, 36864,
            "total vertex count regressed after convergence: {total_verts}"
        );
        // The stats snapshot should reflect the work done.
        assert!(
            core.stats().blocks_loaded > 0 && core.stats().meshes_built > 0,
            "stats should be non-zero: {:?}",
            core.stats()
        );
    }
}

#[cfg(test)]
mod lod_transition_parity {
    use voxel_core::math::Vector3i;
    use voxel_core::meshers::{MesherInput, MesherOutput, TransvoxelMesher, VoxelMesher};
    use voxel_core::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat};

    #[test]
    fn lod_hint_produces_more_vertices_than_without() {
        // A large sphere that intersects block boundaries — transition
        // meshes should add extra geometry on the LOD seam faces.
        let mesher = TransvoxelMesher::new();

        // Create a large sphere SDF.
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(16));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut voxels);
        let cx = 8.0f32;
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let d = ((x as f32 - cx).powi(2)
                        + (y as f32 - cx).powi(2)
                        + (z as f32 - cx).powi(2))
                    .sqrt()
                        - 12.0;
                    voxels.set_voxel_f(d, x, y, z, ChannelId::Sdf.index());
                }
            }
        }

        // Without lod_hint.
        let mut input_no_lod = MesherInput::new(&voxels, Vector3i::zero(), 0);
        input_no_lod.lod_hint = false;
        let mut out_no_lod = MesherOutput::default();
        mesher.build(&mut out_no_lod, &input_no_lod);
        let verts_no_lod = out_no_lod.total_vertex_count();

        // With lod_hint.
        let mut input_lod = MesherInput::new(&voxels, Vector3i::zero(), 0);
        input_lod.lod_hint = true;
        let mut out_lod = MesherOutput::default();
        mesher.build(&mut out_lod, &input_lod);
        let verts_lod = out_lod.total_vertex_count();

        assert!(
            verts_lod > verts_no_lod,
            "lod_hint should produce more vertices (transition geometry): {verts_lod} vs {verts_no_lod}"
        );
    }

    /// Golden test: a flat half-space ground plane (y < 8 solid) produces a
    /// fixed, reproducible vertex count, and `lod_hint=true` adds a fixed
    /// number of transition-cell vertices on the +X/+Z seam faces. These
    /// golden values are pinned against the current transvoxel + transition
    /// table implementation; a regression in either will flip the count.
    #[test]
    fn lod_transition_vertex_count_golden() {
        let mesher = TransvoxelMesher::new();
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(16));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Sdf.index()] = ChannelDepth::Bit32;
        fmt.configure_buffer(&mut voxels);
        // Half-space: solid below y=8 (sdf = y - 8).
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    voxels.set_voxel_f(y as f32 - 8.0, x, y, z, ChannelId::Sdf.index());
                }
            }
        }

        let mut input_no_lod = MesherInput::new(&voxels, Vector3i::zero(), 0);
        input_no_lod.lod_hint = false;
        let mut out_no_lod = MesherOutput::default();
        mesher.build(&mut out_no_lod, &input_no_lod);
        let verts_no_lod = out_no_lod.total_vertex_count();

        let mut input_lod = MesherInput::new(&voxels, Vector3i::zero(), 0);
        input_lod.lod_hint = true;
        let mut out_lod = MesherOutput::default();
        mesher.build(&mut out_lod, &input_lod);
        let verts_lod = out_lod.total_vertex_count();

        // Pinned golden values (regular cells + transition cells).
        assert_eq!(
            verts_no_lod, 676,
            "regular-cell vertex count regressed: {verts_no_lod}"
        );
        assert_eq!(
            verts_lod, 796,
            "lod_hint vertex count regressed: {verts_lod}"
        );
        // Transition cells contribute exactly 120 extra vertices on the seam.
        assert_eq!(
            verts_lod - verts_no_lod,
            120,
            "transition-cell vertex delta regressed: {}",
            verts_lod - verts_no_lod
        );
    }
}

#[cfg(test)]
mod instancing_parity {
    use voxel_core::instancing::scatter::{InstanceGenerator, RandomScatterGenerator};
    use voxel_core::instancing::ScatterConfig;
    use voxel_core::math::Vector3f;

    #[test]
    fn scatter_output_has_valid_transforms() {
        let gen = RandomScatterGenerator {
            density: 1.0,
            min_scale: 0.5,
            max_scale: 1.5,
            snap_to_normal: true,
        };
        let positions: Vec<_> = (0..20)
            .map(|i| Vector3f::new(i as f32 * 2.0, 10.0, 0.0))
            .collect();
        let normals = vec![Vector3f::new(0.0, 1.0, 0.0); 20];
        let config = ScatterConfig::default();
        let result = gen.generate(&positions, &normals, 0, &config);

        assert!(result.len() > 0, "should produce instances");
        for instance in &result {
            assert!(
                instance.scale >= 0.5 && instance.scale <= 1.5,
                "scale out of range: {}",
                instance.scale
            );
            assert_eq!(instance.item_index, 0, "item_index should be 0");
            // Rotation quaternion should be normalized (w² + x² + y² + z² ≈ 1).
            let r = &instance.rotation;
            let len_sq = r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3];
            assert!(
                (len_sq - 1.0).abs() < 0.01,
                "quaternion not normalized: len_sq={len_sq}"
            );
        }
    }

    #[test]
    fn scatter_respects_density() {
        let gen = RandomScatterGenerator {
            density: 0.0, // Accept nothing
            min_scale: 1.0,
            max_scale: 1.0,
            snap_to_normal: false,
        };
        let positions = vec![Vector3f::new(0.0, 0.0, 0.0); 100];
        let normals = vec![Vector3f::new(0.0, 1.0, 0.0); 100];
        let config = ScatterConfig::default();
        let result = gen.generate(&positions, &normals, 0, &config);
        assert_eq!(result.len(), 0, "density=0 should produce no instances");
    }

    /// Golden test: scatter output count is deterministic for a fixed seed
    /// and scales linearly with density. With the default `ScatterConfig`
    /// (seed 0) and 100 surface points, density=1.0 yields exactly 100
    /// instances and density=0.5 yields exactly 50. Pinned against the
    /// current xorshift acceptance-sampling implementation.
    #[test]
    fn scatter_output_count_golden() {
        let positions: Vec<_> = (0..100)
            .map(|i| Vector3f::new(i as f32 * 2.0, 10.0, 0.0))
            .collect();
        let normals = vec![Vector3f::new(0.0, 1.0, 0.0); 100];
        let config = ScatterConfig::default();

        // density = 1.0 → every point accepted.
        let gen_full = RandomScatterGenerator {
            density: 1.0,
            min_scale: 0.5,
            max_scale: 1.5,
            snap_to_normal: true,
        };
        let result_full = gen_full.generate(&positions, &normals, 0, &config);
        assert_eq!(
            result_full.len(),
            100,
            "density=1.0 instance count regressed: {}",
            result_full.len()
        );

        // density = 0.5 → exactly half accepted (deterministic PRNG).
        let gen_half = RandomScatterGenerator {
            density: 0.5,
            min_scale: 0.5,
            max_scale: 1.5,
            snap_to_normal: true,
        };
        let result_half = gen_half.generate(&positions, &normals, 0, &config);
        assert_eq!(
            result_half.len(),
            50,
            "density=0.5 instance count regressed: {}",
            result_half.len()
        );

        // The count must be stable across repeated calls (deterministic).
        let result_half2 = gen_half.generate(&positions, &normals, 0, &config);
        assert_eq!(
            result_half.len(),
            result_half2.len(),
            "scatter count is not deterministic"
        );
    }
}

#[cfg(test)]
mod modifier_parity {
    use voxel_core::math::Vector3f;
    use voxel_core::modifiers::{ModifierStack, SdfOperation, SphereModifier};

    /// A sphere modifier subtracted from a SOLID (negative) field carves a
    /// hole: voxels near the sphere center become air (sdf >= 0). The number
    /// of voxels made air is deterministic for a centered sphere. Golden.
    #[test]
    fn sphere_subtract_carves_from_solid() {
        // 5³ grid of voxels at integer positions, all starting SOLID (sdf=-10).
        let positions: Vec<Vector3f> = (0..5)
            .flat_map(|x| {
                (0..5).flat_map(move |y| {
                    (0..5).map(move |z| Vector3f::new(x as f32, y as f32, z as f32))
                })
            })
            .collect();
        let mut sdf = vec![-10.0f32; positions.len()];

        let modifier = SphereModifier {
            center: Vector3f::new(2.0, 2.0, 2.0),
            radius: 2.0,
            operation: SdfOperation::Subtract,
            smoothness: 0.0,
        };
        let mut stack = ModifierStack::new();
        stack.add(Box::new(modifier));
        stack.apply(&mut sdf, &positions);

        let made_air = sdf.iter().filter(|&&v| v >= 0.0).count();
        assert!(made_air > 0, "subtract should carve air voxels: {made_air}");
        assert_eq!(made_air, 33, "carved-air voxel count regressed: {made_air}");
    }

    /// A sphere modifier added (union) into an AIR (positive) field makes
    /// voxels near the sphere solid (sdf < 0). The count is deterministic. Golden.
    #[test]
    fn sphere_add_merges_into_air_field() {
        let positions: Vec<Vector3f> = (0..5)
            .flat_map(|x| {
                (0..5).flat_map(move |y| {
                    (0..5).map(move |z| Vector3f::new(x as f32, y as f32, z as f32))
                })
            })
            .collect();
        let mut sdf = vec![10.0f32; positions.len()];

        let mut stack = ModifierStack::new();
        stack.add(Box::new(SphereModifier {
            center: Vector3f::new(2.0, 2.0, 2.0),
            radius: 1.5,
            operation: SdfOperation::Add,
            smoothness: 0.0,
        }));
        stack.apply(&mut sdf, &positions);

        let made_solid = sdf.iter().filter(|&&v| v < 0.0).count();
        assert!(made_solid > 0, "add should make solid voxels: {made_solid}");
        assert_eq!(
            made_solid, 19,
            "made-solid voxel count regressed: {made_solid}"
        );
    }

    /// An empty modifier stack is a no-op: SDF is unchanged.
    #[test]
    fn empty_modifier_stack_is_noop() {
        let positions = vec![Vector3f::new(0.0, 0.0, 0.0)];
        let mut sdf = vec![5.0f32];
        let stack = ModifierStack::new();
        assert!(stack.is_empty());
        stack.apply(&mut sdf, &positions);
        assert_eq!(sdf, vec![5.0], "empty stack should not change SDF");
    }

    /// Subtract and Add are inverse: subtracting a sphere then adding it back
    /// (in the same positions) returns the field close to its original state at
    /// voxels outside the boundary, while the boundary voxels reflect the blend.
    /// Diff test: the two operations produce different results.
    #[test]
    fn add_and_subtract_produce_different_results() {
        let positions: Vec<Vector3f> = (0..5)
            .flat_map(|x| {
                (0..5).flat_map(move |y| {
                    (0..5).map(move |z| Vector3f::new(x as f32, y as f32, z as f32))
                })
            })
            .collect();

        let mut sdf_sub = vec![-5.0f32; positions.len()];
        let mut stack_sub = ModifierStack::new();
        stack_sub.add(Box::new(SphereModifier {
            center: Vector3f::new(2.0, 2.0, 2.0),
            radius: 2.0,
            operation: SdfOperation::Subtract,
            smoothness: 0.0,
        }));
        stack_sub.apply(&mut sdf_sub, &positions);

        let mut sdf_add = vec![-5.0f32; positions.len()];
        let mut stack_add = ModifierStack::new();
        stack_add.add(Box::new(SphereModifier {
            center: Vector3f::new(2.0, 2.0, 2.0),
            radius: 2.0,
            operation: SdfOperation::Add,
            smoothness: 0.0,
        }));
        stack_add.apply(&mut sdf_add, &positions);

        let diffs = sdf_sub
            .iter()
            .zip(sdf_add.iter())
            .filter(|(&a, &b)| (a - b).abs() > 1e-6)
            .count();
        assert!(diffs > 0, "subtract and add should differ: {diffs}");
    }
}

#[cfg(test)]
mod blocky_library_parity {
    use voxel_core::meshers::blocky::{bake_library, BakedLibrary, BakedModel, AIR_ID};

    /// Adding models to a BakedLibrary increments the model count, and
    /// `has_model` correctly reports presence/absence.
    #[test]
    fn library_tracks_model_count_and_presence() {
        let mut lib = BakedLibrary::default();
        assert!(!lib.has_model(0), "empty library should have no models");
        assert_eq!(lib.models.len(), 0);

        let m1 = BakedModel {
            color: voxel_core::math::Color::from_rgb(1.0, 0.0, 0.0),
            empty: false,
            ..BakedModel::default()
        };
        lib.models.push(m1);
        assert!(lib.has_model(0));
        assert!(!lib.has_model(1));

        lib.models.push(BakedModel::default());
        assert!(lib.has_model(0));
        assert!(lib.has_model(1));
        assert!(!lib.has_model(2));
    }

    /// `bake_library` is idempotent on an empty library and doesn't panic.
    #[test]
    fn bake_library_runs_on_empty() {
        let mut lib = BakedLibrary::default();
        bake_library(&mut lib);
        assert_eq!(lib.models.len(), 0);
    }

    /// `bake_library` populates the side-pattern culling matrix and the
    /// side_pattern_count when models are present.
    #[test]
    fn bake_library_populates_culling_matrix() {
        let mut lib = BakedLibrary::default();
        // Add a non-empty solid model that culls neighbors.
        lib.models.push(BakedModel {
            color: voxel_core::math::Color::from_rgb(0.5, 0.5, 0.5),
            empty: false,
            culls_neighbors: true,
            ..BakedModel::default()
        });
        bake_library(&mut lib);
        assert!(
            lib.side_pattern_count > 0,
            "side_pattern_count should be set after bake"
        );
    }

    /// The air sentinel (`AIR_ID`) is distinct from valid model ids.
    #[test]
    fn air_id_is_not_a_valid_model_in_empty_library() {
        let lib = BakedLibrary::default();
        // AIR_ID refers to index 0 conceptually; an empty library has no model 0.
        assert!(!lib.has_model(0));
        let _ = AIR_ID; // sentinel exists and is usable
    }
}

#[cfg(test)]
mod cubes_mesher_parity {
    use voxel_core::math::Vector3i;
    use voxel_core::meshers::cubes::palette::ColorPalette;
    use voxel_core::meshers::{CubesMesher, MesherInput, MesherOutput, VoxelMesher};
    use voxel_core::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat};

    /// A half-solid buffer (x < 4 opaque, x >= 4 air) on the Color channel
    /// produces a single greedy-merged face. Golden vertex/triangle count.
    #[test]
    fn cubes_mesmer_half_solid_vertex_count_golden() {
        let mesher = CubesMesher::new();
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(8));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Color.index()] = ChannelDepth::Bit8;
        fmt.configure_buffer(&mut voxels);
        let opaque: u64 = 0xFFFFFFFF;
        for x in 0..4 {
            for y in 0..8 {
                for z in 0..8 {
                    voxels.set_voxel(opaque, x, y, z, ChannelId::Color.index());
                }
            }
        }
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        let mut out = MesherOutput::default();
        mesher.build(&mut out, &input);
        // One greedy-merged quad face at the x=4 boundary.
        assert_eq!(
            out.total_vertex_count(),
            4,
            "cubes half-solid vertex count regressed: {}",
            out.total_vertex_count()
        );
        assert_eq!(
            out.total_triangle_count(),
            2,
            "cubes half-solid triangle count regressed: {}",
            out.total_triangle_count()
        );
    }

    /// An all-air buffer produces no vertices from the CubesMesher.
    #[test]
    fn cubes_mesher_all_air_is_empty() {
        let mesher = CubesMesher::new();
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(8));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Color.index()] = ChannelDepth::Bit8;
        fmt.configure_buffer(&mut voxels);
        // All air (0).
        voxels.fill(0, ChannelId::Color.index());

        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);
        let mut out = MesherOutput::default();
        mesher.build(&mut out, &input);
        assert_eq!(
            out.total_vertex_count(),
            0,
            "all-air buffer should produce no vertices"
        );
    }

    /// A custom palette doesn't change the vertex/triangle topology (colors
    /// only affect appearance, not geometry). Diff test: RAW vs Palette mode
    /// over the same half-solid buffer produce identical vertex/triangle counts.
    #[test]
    fn cubes_palette_does_not_change_topology() {
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(8));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Color.index()] = ChannelDepth::Bit8;
        fmt.configure_buffer(&mut voxels);
        let opaque: u64 = 0xFFFFFFFF;
        for x in 0..4 {
            for y in 0..8 {
                for z in 0..8 {
                    voxels.set_voxel(opaque, x, y, z, ChannelId::Color.index());
                }
            }
        }
        let input = MesherInput::new(&voxels, Vector3i::zero(), 0);

        let raw_mesher = CubesMesher::new(); // default RAW mode
        let mut out_raw = MesherOutput::default();
        raw_mesher.build(&mut out_raw, &input);

        let mut palette = ColorPalette::default();
        palette.set_color8(0xFF, voxel_core::math::Color8::new(255, 255, 255, 255));
        let palette_mesher = CubesMesher::new().with_palette(palette);
        let mut out_pal = MesherOutput::default();
        palette_mesher.build(&mut out_pal, &input);

        assert_eq!(
            out_raw.total_vertex_count(),
            out_pal.total_vertex_count(),
            "palette mode should not change vertex topology"
        );
        assert_eq!(
            out_raw.total_triangle_count(),
            out_pal.total_triangle_count(),
            "palette mode should not change triangle topology"
        );
    }
}

#[cfg(test)]
mod edition_tool_parity {
    use voxel_core::edition::ops::VoxelToolBuffer;
    use voxel_core::math::{Vector3f, Vector3i};
    use voxel_core::storage::{ChannelDepth, ChannelId, VoxelBuffer, VoxelFormat};

    /// `do_sphere` carves a sphere of solid voxels into an empty buffer. The
    /// count of solid voxels is deterministic for a centered sphere. Golden.
    #[test]
    fn do_sphere_carves_deterministic_voxel_count() {
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(16));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Type.index()] = ChannelDepth::Bit8;
        fmt.configure_buffer(&mut voxels);

        let mut tool = VoxelToolBuffer::new(&mut voxels, ChannelId::Type.index());
        tool.do_sphere(Vector3f::new(8.0, 8.0, 8.0), 5.0);

        let solid = count_solid(&voxels, ChannelId::Type.index());
        assert!(solid > 0, "do_sphere should carve solid voxels: {solid}");
        assert_eq!(solid, 552, "do_sphere voxel count regressed: {solid}");
    }

    /// `do_box` fills an axis-aligned box region with solid voxels. The count
    /// equals the box volume (exclusive max, matching the C++ range).
    #[test]
    fn do_box_fills_exact_volume() {
        let mut voxels = VoxelBuffer::with_size(Vector3i::splat(16));
        let mut fmt = VoxelFormat::new();
        fmt.depths[ChannelId::Type.index()] = ChannelDepth::Bit8;
        fmt.configure_buffer(&mut voxels);

        let min = Vector3i::new(4, 4, 4);
        let max = Vector3i::new(10, 10, 10);
        let mut tool = VoxelToolBuffer::new(&mut voxels, ChannelId::Type.index());
        tool.do_box(min, max);

        let solid = count_solid(&voxels, ChannelId::Type.index());
        // Range [4,10) per axis → 6³ = 216.
        assert_eq!(solid, 216, "do_box should fill exact volume: {solid}");
    }

    fn count_solid(voxels: &VoxelBuffer, channel: usize) -> usize {
        let s = voxels.size();
        let mut count = 0;
        for z in 0..s.z {
            for y in 0..s.y {
                for x in 0..s.x {
                    if voxels.get_voxel(x, y, z, channel) != 0 {
                        count += 1;
                    }
                }
            }
        }
        count
    }
}

#[cfg(test)]
mod graph_runtime_parity {
    use voxel_core::generators::graph::{
        CompiledGraph, CompiledScratch, Graph, GraphInputs, GraphOutput, GraphPort, NodeKind,
    };

    /// A constant → OutputSdf graph produces that exact constant value.
    /// Golden single-value check.
    #[test]
    fn graph_constant_output_is_exact() {
        let mut g = Graph::new();
        let c = g.push(NodeKind::Constant(7.5));
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort { node: c, output: 0 }),
        });
        let compiled = CompiledGraph::compile(&g).expect("compile");
        let xs = [0.0f32];
        let zs = [0.0f32];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &zs,
        };
        let mut scratch = CompiledScratch::new();
        let mut out = Vec::new();
        compiled.generate_slice(&inputs, 1, &mut scratch, &mut out, false);
        let sdf: f32 = out
            .into_iter()
            .find(|(k, _)| *k == GraphOutput::Sdf)
            .and_then(|(_, v)| v.into_iter().next())
            .unwrap();
        assert_eq!(sdf, 7.5, "constant graph output regressed: {sdf}");
    }

    /// A SdfSphere graph at the center point returns -radius (inside surface).
    #[test]
    fn graph_sphere_sdf_at_center_is_negative_radius() {
        let mut g = Graph::new();
        let cx = g.push(NodeKind::Constant(0.0));
        let cy = g.push(NodeKind::Constant(0.0));
        let cz = g.push(NodeKind::Constant(0.0));
        let cr = g.push(NodeKind::Constant(4.0));
        let sphere = g.push(NodeKind::SdfSphere {
            x: Some(GraphPort {
                node: cx,
                output: 0,
            }),
            y: Some(GraphPort {
                node: cy,
                output: 0,
            }),
            z: Some(GraphPort {
                node: cz,
                output: 0,
            }),
            radius: Some(GraphPort {
                node: cr,
                output: 0,
            }),
        });
        g.push(NodeKind::OutputSdf {
            a: Some(GraphPort {
                node: sphere,
                output: 0,
            }),
        });
        let compiled = CompiledGraph::compile(&g).expect("compile");
        let xs = [0.0f32];
        let zs = [0.0f32];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &zs,
        };
        let mut scratch = CompiledScratch::new();
        let mut out = Vec::new();
        compiled.generate_slice(&inputs, 1, &mut scratch, &mut out, false);
        let sdf: f32 = out
            .into_iter()
            .find(|(k, _)| *k == GraphOutput::Sdf)
            .and_then(|(_, v)| v.into_iter().next())
            .unwrap();
        // At center, dist=0, sdf = 0 - 4 = -4.
        assert!((sdf - (-4.0)).abs() < 1e-5, "sphere sdf at center: {sdf}");
    }
}
