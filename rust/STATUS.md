# Rust Port — Status Snapshot

> Branch: `rust/pilot` · Last update: 2026-07-06
> See `MIGRATION_PLAN.md` at the repo root for the full plan + progress log.

## At a glance

| Phase | Status | Tests |
|---|---|---|
| 0 — Pilot (transvoxel mesher + cross-compile) | 🟡 CONDITIONAL GO (H1 byte-parity partial) | H2 pass; H1 partial |
| 1 — Pure core (`util/{math,string,memory,io,testing}` + `expression_parser`) | ✅ COMPLETE | (cumulative) |
| 2 — Mobile validation (gdext `.so` desktop + Android) | ✅ desktop+Android `.so` (on-device: pending SDK) | — |
| 3 — Compute layer (storage, streams, meshers, generators, format) | ✅ COMPLETE | (cumulative) |
| 4 — Terrain + threading (storage/streaming/meshing/paging/graph) | 🟡 IN PROGRESS | 625 unit + 10 integration |
| 5 — Godot binding + editor | ⏳ not started | — |

**Total:** 625 unit tests + 10 integration + 1 doc-test, clippy clean.

## Phase 4 — what works headlessly (no Godot)

The full pipeline runs end-to-end in pure Rust, no engine dependency:

```
GraphGenerator (24+ node kinds: SDF/Curve/Noise/math/IO)
   │  or  Waves / Flat / Noise / HeightmapNoise (simple generators)
   ▼
VoxelData
   • LOD maps + format + bounds + streaming flags
   • view_area / unview_area (refcount-pinned block residency)
   • copy / paste / paste_masked (per-block, O(1) writability)
   • update_lods LOD cascade (downscale_to mip-map kernel)
   • generator/stream ownership (SharedVoxelGenerator / SharedVoxelStream)
   • get_voxel generator fallback (generate_single)
   ▼
MeshBlockTask
   • gather_voxels_cpu (3×3×3 neighbours + generator gap-fill)
   • VoxelMesher trait dispatch
   ▼
VoxelMesher  ◂─── TransvoxelMesher  (smooth SDF, regular cells)
            ◂─── CubesMesher       (greedy/simple, palette)
            ◂─── BlockyMesher      (voxel-model library + AO)
   ▼
MesherOutput { surfaces: Vec<Surface>, collision_surface }
   ▼
VoxelTerrainCore (single-LOD paging orchestrator)
   • paired viewers → data/mesh box diffing (+1 mesh-neighbour padding)
   • view_mesh_block / unview_mesh_block (refcount-tracked)
   • try_schedule_mesh_update (has_all_blocks_in_area gate)
   • LoadBlockForTerrainTask (stream first, generator fallback)
   • save-on-unload for modified data blocks
   • full process() tick — viewers → loads → meshing → outputs → unload
```

## Phase 4 — what remains

- **Multi-LOD paging** (`VoxelLodTerrain`): `VoxelLodTerrainUpdateData` + threaded update task + clipbox/octree strategy (~4k lines C++).
- **`VoxelEngine` subset**: `SlotMap` volume/viewer registry + `process()` dequeue loop + `sync_viewers_task_priority_data` for multi-volume scenes.
- **Graph extensions**: Curve/Image range analysis, FastNoise2, Expression node (parser is ported, not wired), bytecode VM optimisation.
- **`VoxelDataGrid`**, **real `SpatialLock3D`** (currently a no-op stub; `&mut self` enforces exclusivity today), **ThreadSanitizer** end-to-end.
- **Phase 5 Godot binding**: `Node3D` wrappers for `VoxelTerrainCore` + `RenderingServer` mesh upload + `EditorPlugin`.

## Crate layout

```
rust/
├── Cargo.toml              # workspace: voxel-core + voxel-gdext
├── rust-toolchain.toml     # pinned toolchain
├── voxel-core/             # engine-agnostic Rust core (NO Godot dep)
│   ├── src/
│   │   ├── math/           # Vector2/3/4i/f, Box2/3i/f, Color, SDF, Quaternion, ...
│   │   ├── storage/        # VoxelBuffer, VoxelData (+LOD cascade, view/unview),
│   │   │                   #   VoxelDataMap, VoxelDataBlock, VoxelFormat, memory pool
│   │   ├── meshers/        # VoxelMesher trait + Transvoxel/Cubes/Blocky adapters,
│   │   │                   #   MeshBlockTask, builtin.rs (adapter wrappers)
│   │   ├── generators/     # VoxelGenerator trait + Graph runtime + simple (Waves/Flat/Noise)
│   │   ├── streams/        # VoxelStream trait, region files, block serializer, instance_data,
│   │   │                   #   load/save tasks, compressed_data (LZ4/ZSTD)
│   │   ├── terrain/        # VoxelTerrainCore (single-LOD paging orchestrator)
│   │   ├── engine/         # PriorityDependency, StreamingDependency, MeshingDependency
│   │   ├── tasks/          # ThreadedTaskRunner (priority throttle), AsyncDependencyTracker
│   │   ├── thread/         # Mutex/BinaryMutex/RwLock, Semaphore, SpatialLock3D (stub)
│   │   ├── format/vox/     # MagicaVoxel .vox parser
│   │   ├── io/             # serialization, text_writer, log, voxel_file, file_locker
│   │   ├── string/         # conv, format, expression_parser (recursive-descent AST)
│   │   └── ...             # constants, containers, hash, memory, testing
│   ├── tests/              # transvoxel parity, sphere, end-to-end pipeline
│   └── benches/            # transvoxel criterion benches
├── voxel-gdext/            # thin Godot binding (Phase 5; loads in Godot 4.7 today)
├── cpp-baseline/           # C++ mesh harness for H1/H2 parity validation
└── scripts/                # android-build.sh (NDK r29 + rust-lld workaround)
```

## Commands

```bash
cd rust
cargo test -p voxel-core       # 610 unit + 10 integration + 1 doc-test
cargo build -p voxel-gdext     # GDExtension .so (loads in Godot 4.7)
cargo clippy --workspace --all-targets   # clean
cargo bench                    # transvoxel benches
./scripts/android-build.sh     # Android aarch64 gdext .so
```
