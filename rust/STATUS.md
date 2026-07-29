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
| 4 — Terrain + threading (storage/streaming/meshing/paging/graph) | ✅ GO (multi-LOD paging M2.1 + transition cells M2.2 + TSan M1.A) | 751 unit + 11 integration + 5 TSan |
| 5 — Godot binding + editor | ⏳ not started | — |

**Total:** 766 unit tests + 497 parity + 5 integration + 1 doc-test, clippy clean.
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
| **M1** | Долг по ревью кода (§9 + §7): TSan, D7, волна 3 (B1/B3/B4/B5/C1/C3), H2-MT бенч, cargo-fuzz, CI, риски §7 | ✅ **ПОЛНОСТЬЮ ЗАКРЫТ 2026-07-12** — M1.A (TSan) + M1.B (D7) + M1.C (волна 3) + M1.D (graph C1+C3) + M1.E (cargo-fuzz + OOM fix + §7 риски + H2-MT bench). CI auto-trigger (item 11) отложено до стабилизации пилота. |
| **M2** | Фаза 4 до GO: multi-LOD paging (`VoxelLodTerrain`), остаток `VoxelEngine`, `VoxelDataGrid`, сквозной TSan | ✅ **GO-критерий закрыт:** M2.1 (multi-LOD paging) + M2.2 (transition cells) + TSan (M1.A). Clipbox/fading/VoxelEngine residual — deferred polish. |
| **M3** | Фаза 5: Godot binding 75+ классов + editor/edition/modifiers/instancing/terrain-root | 🟡 в работе: 9 Godot classes (terrain+viewer+4 generators+2 streams+VoxelToolBufferGD) + edition/ (ops+raycast) + modifiers/ (trait+sphere+stack). Остаются: instancing (~9.2k), editor plugins (~12.8k) |
| **M4** | Паритет и удаление C++ из `master`; форк — чистый Rust-проект | ⏳ |

**Текущий фокус:** **M1 полностью закрыт.** Все основные пункты выполнены: TSan (0 data race),
typed storage (D7), wave 3 mesher perf (B1+B3+B4+B5), graph compile-step + range analysis (C1+C3),
cargo-fuzz (3 таргета + найден/починен OOM bug), H2-MT bench (2.25× MT speedup), §7 риски.
H2-MT bench результаты: single 47µs/86 Melem/s, multi 673µs/194 Melem/s (4 потока).
**M2 GO-критерий закрыт.** Multi-LOD paging (M2.1) + transition cells (M2.2) + TSan (M1.A) = Фаза 4
выполнена. Clipbox streaming / fading / VoxelEngine residual deferred как polish.
**Следующий milestone — M3 (Phase 5):** Godot binding + editor.
**M3 в работе:** VoxelTerrain (Node3D) + VoxelViewer (Node3D) + 4 generator Resources
(Waves/Flat/Noise/Heightmap) + edition tools (set/get_voxel_sdf, raycast) +
lod_count property + dirty block re-upload + material_override + generate_collision.
Rendering, editing, materials, collision — функциональны в Godot.
Далее: stream binding (save/load), editor plugins, instancing, modifiers.

**D7 (M1.B):** `Channel.data` теперь `enum ChannelData { U8/U16/U32/U64(Vec<_>) }` — hot loops
depth-dispatch один раз на канал и индексируют типизированный slice напрямую. Wire-format
`block_serializer` неизменен (`bytemuck` safe byte-cast). Pool recycling для typed storage отложен
(test-only). Новая runtime-dep: `bytemuck = "1"`.

**TSan-прогон** (M1.A): `cargo +nightly test -p tsan -Zbuild-std --target x86_64-unknown-linux-gnu`
с `RUSTFLAGS="-Zsanitizer=thread -Cunsafe-allow-abi-mismatch=sanitizer"` — 5 тестов, стабильно 0
data race. Workspace-член `tsan` изолирован от `criterion`/`zerocopy` (proc-macro конфликтует с
TSan-runtime). `-Zbuild-std` обязателен — иначе std не инструментирован и TSan выдаёт false positives.

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

- **Multi-LOD paging** (`VoxelLodTerrain`): ✅ M2.1 (LodOctree + 2 LOD paging + per-LOD dispatch)
  + M2.2 (transition cells) closed. Clipbox streaming (multi-viewer, ~1.7k C++ LOC) and LOD
  fading (~500 LOC, Phase 5 shader work) deferred as polish/enhancement.
- **`VoxelEngine` remaining subset**: time-spread/progressive queues, GPU queue, file locker,
  stats/profiling — deferred as infrastructure enhancements (not blocking Phase 4 GO).
- **Concurrency**: ✅ TSan closed (M1.A). SpatialLock3D + per-LOD locks + ThreadedTaskRunner
  semaphore/staging all verified.
- **Graph extensions**: Curve/Image range analysis, FastNoise2, Expression node — optional
  follow-ups (C1+C3 core compile-step already delivers the main perf win).
- **`VoxelDataGrid`**: deferred (alternative storage; existing per-LOD maps suffice).
- **Phase 4 GO criteria**: ✅ multi-LOD paging functional, TSan green, transition cells wired.

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
├── tsan/                   # ThreadSanitizer target crate (isolated from criterion/zerocopy)
├── cpp-baseline/           # C++ mesh harness for H1/H2 parity validation
└── scripts/                # android-build.sh (NDK r29 + rust-lld workaround)
```

## Commands

```bash
cd rust
cargo test -p voxel-core       # 766 unit + 497 parity + 5 integration + 1 doc-test
cargo build -p voxel-gdext     # GDExtension .so (loads in Godot 4.7)
cargo clippy --workspace --all-targets   # clean
cargo bench                    # transvoxel benches
./scripts/android-build.sh     # Android aarch64 gdext .so

# ThreadSanitizer (requires nightly; -Zbuild-std mandatory so std is instrumented)
CARGO_TARGET_DIR=/tmp/tsan-target \
RUSTFLAGS="-Zsanitizer=thread -Cunsafe-allow-abi-mismatch=sanitizer" \
cargo +nightly test -p tsan -Zbuild-std --target x86_64-unknown-linux-gnu -- --test-threads=1
```
