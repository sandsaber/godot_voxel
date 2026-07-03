//! Criterion benchmarks for the transvoxel regular-cell mesher.
//!
//! Phase 0 step 0.8 — the Rust side of the H2 (performance) hypothesis. These
//! measure the steady-state cost of meshing one padded SDF block, the unit of
//! work the engine does per terrain chunk. Throughput is reported as
//! million-cells/sec and vertices produced.
//!
//! A C++ baseline (same SDF input, same algorithm) is produced separately by
//! the C++ reference harness; the per-size numbers here are compared against it
//! in `REPORT.md` to decide the GO/NO-GO perf criterion.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use voxel_core::math::Vector3i;
use voxel_core::meshers::transvoxel::{
    build_regular_mesh, BuildRegularMeshParams, Cache, MeshArrays, RegularMesherInput,
};

/// Transvoxel padding (matches `MIN_PADDING`/`MAX_PADDING` in the port).
const MIN_PADDING: i32 = 1;
const MAX_PADDING: i32 = 2;

/// A raw `f32` SDF volume in the engine's ZXY layout (Y innermost), padded for
/// normal computation. This is the leanest possible `RegularMesherInput`: a flat
/// slice indexed exactly like the mesher expects, with no buffer indirection,
/// so the benchmark measures the mesher, not storage.
struct RawSdfInput {
    data: Vec<f32>,
    block_size: Vector3i,
}

impl RawSdfInput {
    /// Build a padded block of `inner`³ voxels containing an SDF sphere of the
    /// given radius, centered in the inner region. SDF sign convention matches
    /// the integration test: stored value = -(geometric distance), so positive
    /// means "inside solid".
    fn sphere(inner: i32, radius: f32) -> Self {
        let bs = Vector3i::new(
            inner + MIN_PADDING + MAX_PADDING,
            inner + MIN_PADDING + MAX_PADDING,
            inner + MIN_PADDING + MAX_PADDING,
        );
        let sx = bs.x as usize;
        let sy = bs.y as usize;
        let sz = bs.z as usize;
        let mut data = vec![0.0f32; sx * sy * sz];

        let cx = inner as f32 * 0.5;
        let cy = inner as f32 * 0.5;
        let cz = inner as f32 * 0.5;

        for z in 0..sz {
            for y in 0..sy {
                for x in 0..sx {
                    let ix = x as f32 - MIN_PADDING as f32;
                    let iy = y as f32 - MIN_PADDING as f32;
                    let iz = z as f32 - MIN_PADDING as f32;
                    let dist =
                        ((ix - cx).powi(2) + (iy - cy).powi(2) + (iz - cz).powi(2)).sqrt() - radius;
                    // ZXY layout: index = y + sy*(x + sx*z). Y innermost.
                    let i = y + sy * (x + sx * z);
                    data[i] = -dist;
                }
            }
        }
        Self {
            data,
            block_size: bs,
        }
    }

    /// Number of *cells* (2x2x2 voxel groups) the mesher will visit, i.e. the
    /// inner block volume. Used as the throughput unit.
    fn cell_count(&self) -> u64 {
        let inner = self.block_size.x - (MIN_PADDING + MAX_PADDING);
        (inner as u64).pow(3)
    }
}

impl RegularMesherInput for RawSdfInput {
    fn len(&self) -> usize {
        self.data.len()
    }
    fn block_size(&self) -> Vector3i {
        self.block_size
    }
    fn sample_f32(&self, data_index: usize) -> f32 {
        self.data[data_index]
    }
}

/// Mesh one block end-to-end: fresh reuse cache + output, default params.
/// This is the realistic per-chunk cost (the cache is reset inside the call,
/// and a fresh output avoids measuring `Vec::clear` amortization tricks).
fn bench_one_block(input: &RawSdfInput) -> usize {
    let params = BuildRegularMeshParams {
        lod_index: 0,
        edge_clamp_margin: 0.0,
    };
    let mut cache = Cache::default();
    let mut output = MeshArrays::default();
    build_regular_mesh(input, &params, &mut cache, &mut output);
    output.vertices.len()
}

fn bench_regular_mesher(c: &mut Criterion) {
    // inner³ block sizes. 16³ ≈ a typical LOD0 chunk slice; 32³ a full chunk;
    // 64³ stress-tests the hot loop.
    let cases: &[(i32, f32)] = &[(16, 6.0), (32, 13.0), (64, 27.0)];

    let mut group = c.benchmark_group("transvoxel/regular");
    for &(inner, radius) in cases {
        let input = RawSdfInput::sphere(inner, radius);
        // Throughput unit = cells processed (the work the mesher actually does).
        group.throughput(Throughput::Elements(input.cell_count()));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("sphere_{}", inner)),
            &input,
            |b, input| {
                b.iter(|| std::hint::black_box(bench_one_block(input)));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_regular_mesher);
criterion_main!(benches);
