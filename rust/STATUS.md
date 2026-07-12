# Rust Port — Status Snapshot

> Branch: `rust/pilot` · Last update: 2026-07-12
> See `MIGRATION_PLAN.md` at the repo root for the full plan + progress log.
> Roadmap «полностью закрыть аудит» — см. §11 в `rust/AUDIT.md` и блок ниже.

## At a glance

| Phase | Status | Tests |
|---|---|---|
| 0 — Pilot (transvoxel mesher + cross-compile) | ✅ GO | H1/H2 pass |
| 1 — Pure core (`util/{math,string,memory,io,testing}` + `expression_parser`) | ✅ COMPLETE | (cumulative) |
| 2 — Mobile validation (gdext `.so` desktop + Android) | ✅ desktop+Android `.so` (on-device: pending SDK) | — |
| 3 — Compute layer (storage, streams, meshers, generators, format) | ✅ COMPLETE | (cumulative) |
| 4 — Terrain + threading (storage/streaming/meshing/paging/graph) | 🟡 IN PROGRESS | 655 unit + 11 integration |
| 5 — Godot binding + editor | ⏳ not started | — |

**Total:** 655 unit tests + 11 integration + 1 doc-test, clippy clean.
**CI:** automatic Rust workflow is intentionally disabled for now. `.github/workflows/rust.yml`
is manual-only (`workflow_dispatch`) and can run fmt, workspace tests, clippy, workspace build,
and Android aarch64 GDExtension smoke when triggered by hand.

## Roadmap — «полностью закрыть аудит»

> Постановка 2026-07-12: сначала закрыть долг по ревью кода (§9 `AUDIT.md`), затем пройти
> весь путь миграции до конца (вариант охвата 4). Каждый пункт — отдельный коммит + push,
> после milestone обновляется этот файл и журнал `§9.7` (для M1). Полная декомпозиция и DoD —
> в `rust/AUDIT.md` §11.

| Milestone | Суть | Статус |
|---|---|---|
| **M1** | Долг по ревью кода (§9 + §7): TSan, D7, волна 3 (B1/B3/B4/B5/C1/C3), H2-MT бенч, cargo-fuzz, CI, риски §7 | ⏳ следующий |
| **M2** | Фаза 4 до GO: multi-LOD paging (`VoxelLodTerrain`), остаток `VoxelEngine`, `VoxelDataGrid`, сквозной TSan | ⏳ |
| **M3** | Фаза 5: Godot binding 75+ классов + editor/edition/modifiers/instancing/terrain-root | ⏳ |
| **M4** | Паритет и удаление C++ из `master`; форк — чистый Rust-проект | ⏳ |

**Текущий фокус:** переход к M1.A (TSan-прогон на Linux/nightly для `threaded_edit_load_mesh_stress`
и `SpatialLock3D` — формальный GO-критерий Фазы 4 по конкурентности).

**Не входит в DoD** (опционально/отложено, трекается отдельно): GPU-путь (`gpu`/`detail_rendering`/`shaders`),
`sqlite`, `multipass`, FastNoise2/SpotNoise, physics (Rapier).

## Phase 4 — what works headlessly (no Godot)

The full pipeline runs end-to-end in pure Rust, no engine dependency:

```
GraphGenerator (24+ node kinds: SDF/Curve/Noise/math/IO; uniform output compression)
   │  or  Waves / Flat / Noise / HeightmapNoise (simple generators; shared Arc curve)
   ▼
VoxelData
   • SharedVoxelData worker handle (settings lock + per-LOD map RwLocks + SpatialLock3D read/write regions)
   • LOD maps + format + bounds + streaming flags
   • view_area / unview_area (refcount-pinned block residency)
   • copy / paste / paste_masked (per-block, O(1) writability)
   • update_lods LOD cascade (downscale_to mip-map kernel)
   • generator/stream ownership (SharedVoxelGenerator = Arc<dyn VoxelGenerator>, no outer mutex; SharedVoxelStream)
   • get_voxel generator fallback (generate_single)
   ▼
MeshBlockTask
   • gather_voxels_cpu (3×3×3 neighbours + generator gap-fill)
   • SharedVoxelMesher = Arc<dyn VoxelMesher>, no outer mesher mutex
   ▼
VoxelMesher  ◂─── TransvoxelMesher  (smooth SDF, regular cells, uniform fast-path)
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
   • nonblocking process() tick — viewers → enqueue loads/meshing → drain completed outputs → unload
   ▼
VoxelEngine foundation
   • generational volume/viewer registry
   • viewer position/distances/flags/network peer metadata
   • shared PriorityViewersData sync for task reprioritization
   • owns ThreadedTaskRunner
   • process() drains completed threaded tasks, enqueues follow-ups, applies results
   • async vs async-IO enqueue wrappers (IO uses serial runner mode)
```

## Phase 4 — what remains

- **Multi-LOD paging** (`VoxelLodTerrain`): `VoxelLodTerrainUpdateData` + threaded update task + clipbox/octree strategy (~4k lines C++).
- **`VoxelEngine` remaining subset**: main-thread time-spread/progressive queues, GPU queue, file locker, stats/profiling and volume callback dispatch.
- **Concurrency audit follow-ups**: ThreadSanitizer coverage for the threaded edit/load/mesh path; macOS cargo stress is covered by `threaded_edit_load_mesh_stress`.
- **Infra follow-ups**: re-enable automatic Rust CI when GitHub flow is ready; add H2-MT benchmark smoke and optional x86_64-android emulator smoke.
- **Graph extensions**: Curve/Image range analysis, FastNoise2, Expression node (parser is ported, not wired), bytecode VM optimisation.
- **`VoxelDataGrid`**, **ThreadSanitizer** end-to-end.
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
│   │   │                   #   load/save tasks, compressed_data (LZ4/ZSTD), deferred header flush
│   │   ├── terrain/        # VoxelTerrainCore (single-LOD paging orchestrator)
│   │   ├── engine/         # PriorityDependency, StreamingDependency, MeshingDependency
│   │   ├── tasks/          # ThreadedTaskRunner (priority throttle), AsyncDependencyTracker
│   │   ├── thread/         # Mutex/BinaryMutex/RwLock, Semaphore, SpatialLock3D
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
cargo test -p voxel-core       # 655 unit + 11 integration + 1 doc-test
cargo build -p voxel-gdext     # GDExtension .so (loads in Godot 4.7)
cargo clippy --workspace --all-targets   # clean
cargo bench                    # transvoxel benches
./scripts/android-build.sh     # Android aarch64 gdext .so
```
