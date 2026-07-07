# План миграции godot_voxel → Rust GDExtension

> **Форк:** https://github.com/sandsaber/godot_voxel
> **Upstream:** https://github.com/Zylann/godot_voxel
> **Дата оценки:** 2026-07-03
> **Модель работы:** AI 24/7 + человек-архитектор
>
> **Snapshot статуса:** см. [`rust/STATUS.md`](rust/STATUS.md) — краткая
> сводка фаз + список того, что уже работает headlessly без Godot.

---

## 1. Почему форк — правильное решение

`godot_voxel` живёт и развивается у Zylann (пользовательская база, bugfix'ы, новые
фичи). Полный перепис на Rust означает **расхождение (divergence) с upstream** с первого
дня. Стратегия:

- **Форк как единый source-of-truth.** Весь Rust-код живёт в твоём форке.
- **Регулярный rebase/upstream-merge.** Пока C++ версия не удалена, тянешь fix'ы от Zylann
  в свою C++ ветку, тестируешь их, затем переносишь эквивалент в Rust.
- **Сохраняем C++ ветку до полного паритета.** Это твой эталон для валидации
  Rust-версии (golden tests, diff-тесты, perf-сравнения).
- **Конечная цель:** после Фазы 5 (полный паритет) C++ удаляется из `master`, форк
  становится чистым Rust-проектом. До этого момента C++ остаётся как reference + fallback.

### Git-стратегия

```
sandsaber/godot_voxel
├── master              ← Rust-версия (стабильная, тегируется релизами)
├── cpp-reference       ← C++ upstream (зеркало Zylann/master + твои патчи)
├── rust/migration-*    ← долгоживущие ветки миграции по фазам
└── rust/pilot          ← ветка Фазы 0 (пилот transvoxel)
```

Правила:
1. `cpp-reference` всегда собирается и проходит upstream-тесты.
2. `master` (Rust) релизится только когда проходит parity-тесты против `cpp-reference`.
3. Каждая фаза — отдельная долгоживущая ветка, merge в `master` после GO-критерия.

---

## 2. Архитектура целевого Rust-проекта

### Crate-структура (Cargo workspace)

```
godot_voxel (fork)
├── rust/                          ← Cargo workspace
│   ├── Cargo.toml                 ← workspace root
│   ├── voxel-core/                ← чистое ядро (без Godot)
│   │   ├── math/                  ← Vector3f, Vector3i, color, fixed_array
│   │   ├── containers/            ← StdVector-аналоги
│   │   ├── noise/                 ← FastNoise2 replacement / FFI wrapper
│   │   ├── storage/               ← VoxelBuffer, channels
│   │   ├── meshers/               ← transvoxel, blocky, cubes
│   │   ├── streams/               ← file, region, sqlite
│   │   ├── generators/            ← noise, graph, heightmap
│   │   ├── terrain/               ← VoxelTerrain logic (без Godot node)
│   │   ├── physics/               ← Rapier integration (см. §9)
│   │   └── tasks/                 ← thread pool, task scheduler
│   ├── voxel-gdext/               ← тонкий Godot-binding слой
│   │   ├── src/lib.rs             ← gdext entry point
│   │   ├── src/nodes/             ← VoxelTerrain, VoxelLodTerrain (Node3D wrappers)
│   │   ├── src/resources/         ← VoxelBuffer, VoxelLibrary, VoxelGenerator wrappers
│   │   └── src/editor/            ← EditorPlugin'ы
│   ├── voxel-ffi/                 ← (опционально) C ABI для гибрида
│   └── benches/                   ← критерии perf-сравнения vs C++
├── project/addons/zylann.voxel/   ← Godot-плагин (как сейчас)
├── rust-toolchain.toml            ← pinned toolchain
└── MIGRATION_PLAN.md              ← этот документ
```

### Принцип разделения

```
┌─────────────────────────────────────────────────┐
│  voxel-gdext (тонкий слой)                      │
│  - только Godot-binding: #[func], #[signal]     │
│  - конвертация типов Gd<T> ↔ Rust structs       │
│  - делегирует всю логику в voxel-core           │
└──────────────────────┬──────────────────────────┘
                       │  (вызовы)
┌──────────────────────▼──────────────────────────┐
│  voxel-core (чистый Rust)                       │
│  - не знает про Godot                           │
│  - тестируется без Godot (cargo test)           │
│  - бенчмарки (cargo bench)                      │
│  - можно переиспользовать вне Godot             │
└─────────────────────────────────────────────────┘
```

Это **критично** для Web-экспорта в будущем и для тестируемости. Весь hot-path
живёт в `voxel-core` и не платит за FFI на каждом вызове.

---

## 3. Фаза 0 — Пилот (1-2 недели)

### Цель
Доказать, что стек Rust+gdext работает в этом домене с приемлемой скоростью.
Принять решение GO/NO-GO на основе **измерений**, не впечатлений.

### Что валидируем (4 гипотезы)
1. **H1 — Эквивалентность:** Rust transvoxel выдаёт идентичный mesh C++ версии.
2. **H2 — Производительность:** Rust не медленнее C++ более чем на 15%
   (с target = не медленнее вообще, tolerance 15%).
3. **H3 — Тулинг:** cargo+gdext собирается на desktop без боли.
4. **H4 — Кросс-компиляция:** статическая Rust-библиотека компилируется под
   aarch64-linux-android (NDK). Сам gdext пока не нужен — только `voxel-core`.

### Объект пилота
`meshers/transvoxel/` — алгоритм Marching Cubes с LOD-переходами
(Transvoxel by Eric Lengyel).

**Почему transvoxel:**
- ~2000 строк чистого compute (без Godot, почти без threads)
- Ядро алгоритма в `transvoxel.cpp` (1555 строк) + `transvoxel_tables.cpp` (1081 строк lookup-таблиц)
- Чётко определённый контракт: `(VoxelBuffer, params) → MeshArrays`
- Легко сравнить побайтово с C++

### Зависимости пилота от voxel-core
Переносятся минимально:
- `util/math/vector3f.h`, `vector3i.h`, `color.h` → `voxel-core/math/`
- `util/containers/fixed_array.h` → `voxel-core/containers/`
- `storage/voxel_buffer.h` (минимальный интерфейс чтения SDF-канала) → `voxel-core/storage/`
- `StdVector<T>` → заменить на `Vec<T>` из std

### Шаги Фазы 0

| # | Шаг | Результат | Артефакт |
|---|---|---|---|
| 0.1 | Склонировать форк, создать ветку `rust/pilot` | workspace | git branch ✅ |
| 0.2 | Инициализировать Cargo workspace в `rust/` | компилируемый пустой crate | `cargo build` ✅ |
| 0.3 | Перевести `util/math/*` (Vector3f/i, Color, FixedArray) | unit-тесты | `cargo test math` ✅ |
| 0.4 | Перевести минимальный `VoxelBuffer` (только SDF-чтение) | unit-тесты | `cargo test storage` ✅ |
| 0.5 | Перевести `transvoxel.cpp` + lookup-таблицы | компилируется | `cargo build` ✅ |
| 0.6 | Интеграционный тест: SDF-сфера → mesh | mesh генерируется | `cargo test` ✅ |
| 0.7 | **Parity-тесты:** Rust vs C++ на эталонных данных | golden mesh files совпадают | ⏳ |
| 0.8 | **Бенчмарки:** `criterion` vs C++ baseline | perf report | ⏳ |
| 0.9 | **Cross-compile на Android NDK** | `.a` под aarch64 | ⏳ |
| 0.10 | Документировать результаты, решение GO/NO-GO | REPORT.md | ⏳ |

### Критерий GO/NO-GO (Фаза 0)

**GO если:**
- ✅ H1: mesh идентичен C++ (побайтово или с tolerance на float reordering)
- ✅ H2: perf не хуже C++ более чем на 15%
- ✅ H3: билд воспроизводим на чистой машине за <30 минут
- ✅ H4: `.a` под Android собран без хаков

**NO-GO если:**
- ❌ Perf хуже более чем на 25% без понятной причины → исследовать, возможно retry
- ❌ Parity-тесты не проходят после разумных усилий → алгоритм слишком завязан на C++ UB
- ❌ Кросс-компиляция требует неадекватных хаков → переоценить мобильную стратегию

---

## 4. Фазы 1-5 — основной объём

> Подробности каждой фазы расписываются ПОСЛЕ GO-решения Фазы 0.
> Здесь — скелет с критериями.

### Фаза 1: Чистое ядро (3-4 недели) — ✅ ЗАВЕРШЕНА
- `util/math`, `containers`, `string`, `io`, `memory` полностью
- `util/testing` (фреймворк для parity-тестов)
- **GO-критерий:** ✅ все unit-тесты проходят (191 → 439 cumulative), clippy/fmt чист

### Фаза 2: Мобильная валидация (2-3 недели) — ✅ desktop+mobile `.so` ЗАВЕРШЕН
- Cargo targets: `aarch64-linux-android`, `x86_64-linux-android` ✅
- Минимальный gdext "hello world" ✅ (грузится в Godot 4.7 на desktop)
- `voxel-gdext` Android `.so` собран (aarch64 + x86_64 через NDK r29) ✅
- **GO-критерий:** ⏳ APK с Rust-gdext запускается на Android — нужен custom export
  template + SDK + устройство/эмулятор (вне данного окружения)

### Фаза 3: Compute-слой (6-8 недель) — ✅ ЗАВЕРШЕНА
- `storage` (VoxelBuffer полный), `streams` (instance_data, compressed_data,
  block_serializer, region, stream_cache, stream_memory) ✅
- `meshers` (transvoxel, cubes greedy, **blocky полный**: bake+mesher+skirts+shadow) ✅
- `generators` (Waves, Flat, Noise 3D, HeightmapNoise — через `fastnoise-lite` pure Rust) ✅
- `format::vox` (MagicaVoxel), `constants::cube_tables`, `io` extensions ✅
- **GO-критерий:** ✅ генерация + meshing работают (439 unit тестов); ⏳ end-to-end
  desktop/Android demo — нужен Phase 4 terrain node для интеграции

### Фаза 4: Terrain + threading (8-10 недель) — SINGLE-LOD PAGING TERRAIN ЗАВЕРШЕН (HEADLESS)
- `util/thread` wrappers ✅; `util/tasks` value types + thread runner ✅ (priority throttle 32ms ✅); `io::file_locker` ✅; `VoxelStream` base ✅
- stream dependency shims ✅; voxel-only load/save block tasks ✅ (stream-error/abort output fix ✅); generator/VoxelData integration ✅
- `VoxelBuffer::downscale_to` ✅; `VoxelData::update_lods` LOD cascade ✅
- `VoxelDataBlock::viewers` + `view_area`/`unview_area` ✅
- `VoxelData` generator/stream ownership (`SharedVoxelGenerator = Arc<dyn VoxelGenerator>`, `SharedVoxelStream`) ✅
- `VoxelData` copy/paste/paste_masked + area queries ✅; `paste_masked` per-block + O(1) writability ✅
- `VoxelGenerator::generate_single` + `get_voxel` generator fallback ✅
- `Semaphore` + real `SpatialLock3D` ✅ (region read/write exclusion by overlapping `BoxBounds3i`)
- **`VoxelMesher` trait + MesherInput/MesherOutput/SurfaceArrays enum ✅** (`build(&self)`, shared via `Arc<dyn VoxelMesher>`)
- **`MeshingDependency` ✅; `MeshBlockTask` (gather_voxels_cpu + build) ✅**
- **`TransvoxelMesher` real adapter ✅ (обёртка над build_regular_mesh)**
- **End-to-end test: generator → VoxelData → MeshBlockTask → TransvoxelMesher → mesh ✅**
- **`VoxelTerrainCore` single-LOD paging orchestrator ✅** (viewers → loads → meshing → outputs → unload; full lifecycle работает headlessly)
- **`string::expression_parser` ✅** (closure Фазы 1, открывает generators::graph compilation)
- **`generators::graph` runtime minimal ✅** (AST-walker: InputX/Y/Z, Constant, Add/Sub/Mul/Div, Sin/Cos/Abs/Sqrt, Min/Max, Remap, OutputSdf + GraphGenerator impl VoxelGenerator)
- Multi-LOD paging (VoxelLodTerrain) — далее (clipbox/octree стратегии)
- `VoxelEngine` foundation + task drain loop (volume/viewer registry + shared priority viewers + threaded task ownership/dequeue) ✅
- generators::graph extensions: Curve/Image/Noise/SDF nodes, FastNoise2, range analysis, Expression node, bytecode VM — частично (SDF/Curve/Noise/math nodes готовы; Image/FastNoise2/range analysis/Expression/bytecode VM — далее)
- Cubes/Blocky mesher adapters ✅ (TransvoxelMesher + CubesMesher + BlockyMesher — все три impl VoxelMesher)
- `VoxelDataGrid`, `VoxelData` per-LOD `RwLock` + `SpatialLock3D` integration — далее
- **GO-критерий:** стриминг бесконечного terrain'а работает, нет race conditions
  (проверка под ThreadSanitizer/loom)

  **План порта Phase 4 (порядок):**
  1. `util/thread/{mutex,rw_lock}` — ✅ wrappers over `std::sync` (recursive `Mutex`, `BinaryMutex`, `RwLock`)
  2. `util/tasks/{task_priority,cancellation_token,threaded_task,threaded_task_runner}` — ✅ value types + minimal owned runner
  3. `io::file_locker` — ✅ per-path read/write coordination (depends on mutex)
  4. `streams::{voxel_stream base, load/save_block_data_task}` — ✅ base trait + dependency shims + voxel-only load/save task layer
  5. `engine/{voxel_data,voxel_lod_terrain,voxel_terrain}` — streaming terrain core
  6. `generators/graph` runtime (needs `string::expression_parser` from Phase 1 deferred)
  7. Integration: VoxelTerrain node streaming + LOD + meshers end-to-end

### Фаза 5: Godot-binding + Editor (6-8 недель)
- 75+ классов на gdext (`#[func]`, `#[signal]`, `#[base]`)
- Editor plugins (gizmos, inspector, docks, graph-редактор)
- **GO-критерий:** полный паритет с C++, C++ можно удалить из `master`

### Фаза 6 (опционально): Web (8+ недель)
- Перепроектирование threading под WASM (cooperative/async модель)
- Custom Godot web template
- Ограничение gdext: 1 extension на билд
- **Предусловие:** решение стоит ли Web стоимости отдельной задачи

---

## 5. Технические риски и митигации

| Риск | Вероятность | Влияние | Митигация |
|---|---|---|---|
| Threading + `Gd<T>` lifetime на gdext | Высокая | Высокое | Рано спроектировать ownership-модель; пилот Фазы 0 должен задеть boundary; пилот ThreadSanitizer на Фазе 4 |
| Web не работает (architectural) | Определённая | Среднее | Не обещать Web в основном плане; отдельная Фаза 6 с перепроектированием |
| gdext pre-1.0 ломающие изменения | Высокая | Среднее | Пинить версию; бюджет 1-2 недели на каждую миграцию; держать binding-слой тонким |
| FastNoise2 (15 файлов, C++) | Средняя | Среднее | Сначала FFI wrapper (быстро), потом нативный Rust-порт если FFI-оверхед критичен |
| SIMD perf-parity (sdf, mixel4, fast_noise) | Средняя | Среднее | `std::simd` / `wide` крейт; бенчмарки с Фазы 0 |
| Custom export template для mobile | Определённая | Низкое | Настроить один раз на Фазе 2, переиспользовать; задокументировать |

---

## 6. Метрики успеха (что меряем с Фазы 0)

| Метрика | Где | Цель |
|---|---|---|
| Parity-тесты (mesh byte-equality) | каждый компонент | 100% pass |
| Perf vs C++ (criterion benchmarks) | benches/ | ≥ -15% (target = 0%) |
| Coverage voxel-core | cargo tarpaulin | ≥ 70% к Фазе 3 |
| Cross-compile targets | CI | android ✓ (Ф2), ios ✓ (Ф2), wasm ⬜ |
| Build time на CI | GitHub Actions | <15 мин incremental |
| Количество переведённых строк | tracking | cumulative по фазам |

---

## 7. Прогресс пилота (live)

| Дата | Шаг | Статус |
|---|---|---|
| 2026-07-03 | 0.1-0.4: workspace + math + storage | ✅ 21/21 тестов проходят, clippy чист |
| 2026-07-03 | 0.5-0.6: transvoxel mesher (regular-cell) | ✅ 29/29 тестов проходят, mesh генерируется на SDF-сфере |
| 2026-07-03 | 0.8: criterion benches (16³/32³/64³) | ✅ Rust floor: 147–238 Melem/s. C++ compare pending |
| 2026-07-03 | 0.9: Android NDK cross-compile (H4) | ✅ `.a` + `.so` для aarch64/x86_64-android; +Apple arm64 `.a` с Linux |
| 2026-07-03 | 0.7: parity framework + initial Rust self-consistency golden | ✅ GoldenMesh JSON + comparator, sphere_16/32; superseded by C++ goldens on 2026-07-06 |
| 2026-07-03 | 0.7 (real C++): table parity | ✅ Rust-таблицы byte-identical upstream C++ (`transvoxel_tables_parity`) |
| 2026-07-03 | 0.10: REPORT.md | ✅ Initial GO/NO-GO report; superseded by H1 pass update on 2026-07-06 |
| 2026-07-04 | 0.7 (mesh parity vs C++) + 0.8 (C++ perf baseline) | ✅ C++ harness (без godot-cpp, через stub-tree). Первичный прогон выявил divergence в H1, H2 PASS: Rust 28.5µs/143Melem/s vs C++ 44.1µs/93Mvoxels/s (~1.5× быстрее). Детали в `rust/cpp-baseline/README.md` |
| 2026-07-04 | Фаза 2 mobile-half: voxel-gdext Android `.so` (NDK r29) | ✅ `libvoxel_gdext.so` собран для aarch64 (3.2 MB) **и** x86_64-android (3.2 MB, эмулятор), оба экспортируют `gdext_rust_init`. `rust/scripts/android-build.sh` расширен: дефолт — gdext `.so`, `--core` — voxel-core; `CC`/`CXX` пробрасываются в `godot-cpp`. `.gdextension.in` дополнен `android.x86_64` |
| 2026-07-04 | Фаза 3: `format::vox` (MagicaVoxel `.vox` парсер) | ✅ +23 теста, total 244. `streams/vox/vox_data.{h,cpp}` → `voxel-core/src/format/vox/{data,parser,tests}.rs`. Чистый Rust, ноль новых зависимостей; `&[u8]` cursor вместо `FileAccess`, `Node` enum вместо C++ inheritance, rotation-byte→Basis3f decode с fallback для out-of-spec байт |
| 2026-07-04 | Фаза 3: `streams::instance_data` + io fallible API | ✅ +13 тестов, total 257. `streams/instance_data.{h,cpp}` → `voxel-core/src/streams/instance_data.rs`. Расширение `MemoryReader`: `try_get_*`/`try_take` (Option, без panic) + `set_endianness` для v0 big-endian backcompat. `DeserializeError` enum, round-trip с quantization tolerance |
| 2026-07-04 | Фаза 3: `streams::compressed_data` (LZ4/ZSTD) | ✅ +12 тестов, total 269. `streams/compressed_data.{h,cpp}` → `voxel-core/src/streams/compressed_data.rs`. **Первая runtime-зависимость** voxel-core: `lz4_flex` (pure Rust, без C) для LZ4/LZ4_BE, `zstd` под optional feature. Android gdext `.so` перепроверен (aarch64 + x86_64) — собирается с новой зависимостью. `cargo rustc --crate-type staticlib` упирается в cargo#9562 (задокументировано в `android-build.sh`), но production-артефакт `.so` работает |
| 2026-07-04 | Фаза 3: `streams::block_serializer` (VoxelBuffer↔bytes) | ✅ +11 тестов, total 280. `streams/voxel_block_serializer.{h,cpp}` → `voxel-core/src/streams/block_serializer.rs`. v4-формат (version+size+8 каналов+trailing magic), `serialize_and_compress`/`decompress_and_deserialize` обёртки. Расширения: `MemoryReader::try_get_64`, `VoxelBuffer::set_channel_depth`. **Metadata-секция и v2/v3 legacy-миграция отложены** — завязаны на Godot Variant/custom-metadata factory (`storage/metadata/`, не портирован). Streams-стек (instance_data→compressed_data→block_serializer) завершён |
| 2026-07-04 | Фаза 3: `generators::simple` (Waves + Flat) | ✅ +14 тестов, total 294. `generators/voxel_generator.h` + `generators/simple/{waves,flat}.{h,cpp}` → `voxel-core/src/generators/{base,simple}.rs`. `VoxelGenerator` trait, `generate_heightmap` generic helper. C++ Godot `Resource`+`RWLock`+GPU/cache API опущены (threading в Фазе 4). Тесты: height-функция, bounded range, pattern offset/size, SDF gradient+iso_scale, blocky fill, early-exit ветки, used_channels_mask, range-remap |
| 2026-07-04 | Фаза 3: `generators::simple::Noise` (3D SDF) | ✅ +7 тестов, total 301. `generators/simple/voxel_generator_noise.{h,cpp}` → `generators/simple.rs::Noise`. **Вторая runtime-зависимость**: `fastnoise-lite` v1.1.1 (pure Rust, bit-совместим с Godot FNL). Свой per-voxel 3D loop `(noise_3d+bias)*noise_period` (не heightmap). Тесты: early-exit sentinels ±100, deterministic-same-seed, sign-change-in-slab, blocky 0/1 |
| 2026-07-04 | Фаза 3: `meshers::cubes` (palette + greedy) | ✅ +12 тестов, total 313. `meshers/cubes/{voxel_color_palette,voxel_mesher_cubes}.{h,cpp}` → `voxel-core/src/meshers/cubes/{palette,arrays,greedy,simple}.rs`. Greedy cube meshing (binary face-culling + rectangle merge), `ColorPalette`, opaque/transparent material split. **Отложено**: atlased mode (UV packing), `VoxelMesher` interface |
| 2026-07-04 | Фаза 3: `streams::region` + `io::voxel_file` | ✅ +24 теста, total 337. `streams/region/region_file.{h,cpp}` → `voxel-core/src/streams/region/{format,region_file,mod}.rs` + `io::voxel_file.rs` (`VoxelFile` trait + `StdVoxelFile`/`MemoryFile`). Region-file `.vxr` archive: header/LUT, sector allocator (append/compact/truncate), `load_block`/`save_block` через `block_serializer`. **Отложено**: forest wrapper (meta.vxrm/LRU), v2→v3 migration (needs `insert_bytes`), file locking |
| 2026-07-04 | Фаза 3: blocky mesher (полный portable core) | ✅ +53 теста, total 391. `constants/cube_tables` + `meshers/blocky/{baked_library,bake,mesher,lod_skirts,shadow_occluders}`. Полный blocky meshing: side-culling bake pass (rasterization + pattern dedup + occlusion matrix), `generate_mesh` с AO + face culling, LOD skirts, shadow occluders. Godot Resource/editor слой → Фаза 5 |
| 2026-07-04 | Фаза 3: streams cache/memory + HeightmapNoise — **ФАЗА 3 ЗАВЕРШЕНА** | ✅ +20 тестов, total 411. `streams/{stream_cache,stream_memory}` (engine-agnostic, без Mutex), `generators::simple::HeightmapNoise` (2D noise + Curve через `generate_heightmap`). Все engine-agnostic Phase 3 компоненты портированы |
| 2026-07-05 | Фаза 1-3 audit fixes + старт Фазы 4 | ✅ total 439 unit. Исправлены parity gaps: `shift_up(pos=len)`, `is_uniform([])`, base10 parse без цифр, `%g`-style float formatting, SDF/mixel4 defaults + lower-case channel names, `VoxelFormat::configure_buffer`, region sector compaction, simple-cubes padding coords, Noise frequency, metadata envelope validation, `VoxelBuffer` depth reset/pool-safe channel copy. Фаза 4: `thread::{Mutex,BinaryMutex,RwLock}` + `tasks::{TaskPriority,TaskCancellationToken}` |
| 2026-07-05 | Фаза 4 audit + fixes + оптимизации + миграция | ✅ total 533 unit. Audit выявил 2 бага (stream-error/abort paths теряли output, clippy large_enum_variant) и 2 оптимизации (paste O(N·k)→O(N), ThreadedTaskRunner priority throttle 32ms). Миграция критических блокеров: `VoxelBuffer::downscale_to` (mip-map kernel), `VoxelData::update_lods` LOD cascade, `VoxelDataBlock::viewers` + `view_area`/`unview_area` (refcount pinning API для mesh tasks), `VoxelData` generator/stream ownership (`SharedVoxelGenerator`/`SharedVoxelStream`, `with_generator` helper), `VoxelData` copy/paste/paste_masked + area queries (`is_area_loaded`, `has_all_blocks_in_area`, `get_missing_blocks`, `get_blocks_with_voxel_data`). VoxelGenerator trait → `Send + Sync`. 8 коммитов, clippy чист |
| 2026-07-05 | Фаза 4 meshing pipeline + end-to-end | ✅ total 560 unit + 5 e2e. Meshing pipeline полностью работает headlessly: `generate_single`/`get_voxel` fallback, `Semaphore` + initial `SpatialLock3D` compatibility API (заменён real lock 2026-07-07), `VoxelMesher` trait + `MesherInput`/`MesherOutput`/`SurfaceArrays` enum (transvoxel/cubes/blocky variants), `MeshingDependency`, `MeshBlockTask` (CPU gather+build алгоритм), `TransvoxelMesher` real adapter. End-to-end тест: generator → VoxelData → MeshBlockTask → mesher → MesherOutput — SDF-сфера даёт non-empty mesh, мирный блок далеко от сферы даёт empty mesh, dependency invalidation даёт dropped output. 7 коммитов, clippy чист |
| 2026-07-05 | Фаза 4 single-LOD paging terrain | ✅ total 564 unit + 5 e2e. Engine-agnostic port of `terrain/fixed_lod/voxel_terrain.cpp` paging loop: `VoxelTerrainCore` orchestrator со paired viewers, data/mesh box computation с +1 padding для meshing neighbours, view/unview refcount-tracked mesh blocks, `try_schedule_mesh_update` (has_all_blocks_in_area gate), `LoadBlockForTerrainTask` (stream + generator fallback), full `process()` tick — viewers → loads → meshing → outputs. Тест доказывает полный lifecycle: viewer появляется → блоки грузятся и мешаются → viewer уходит → блоки выгружаются. `Box3i::difference` (slab decomposition для box diffs). Без Godot (Node3D/RenderingServer — Phase 5), без instancer/multiplayer/GPU. Multi-LOD (VoxelLodTerrain) — отдельный orchestrator далее |
| 2026-07-05 | Фаза 1 closure: `string::expression_parser` | ✅ total 578 unit + 5 e2e. Закрыт последний отложенный пункт Фазы 1 — `util/string/expression_parser.{h,cpp}` (~980 LOC C++) → Rust. Recursive-descent parser с operator-precedence stack, AST `enum Node { Number/Variable/Operator/Function }` с `Box<Node>` children (idiomatic Rust, без `Box<dyn>`), `precompute_constants` constant-folding, `find_variables`, `tree_to_string`, `is_tree_equal`. Открывает `generators::graph` runtime (graph compiler lowering). 14 тестов покрывают parsing + folding + error paths + variable extraction + structural compare |
| 2026-07-05 | Фаза 4: `generators::graph` runtime | ✅ total 593 unit + 5 e2e. Engine-agnostic runtime для procedural graph generator: AST-walker интерпретатор (`Graph` topology + `NodeKind` enum с InputX/Y/Z, Constant, Add/Sub/Mul/Div, Sin/Cos/Abs/Sqrt, Min/Max, Remap, OutputSdf), topological sort с cycle detection, `GraphGenerator` impl `VoxelGenerator` (Y-slice loop, SDF output). 15 тестов покрывают topology, evaluation, cycle detection, defaults, generator adapter. Подход проще C++ bytecode VM (быстрее в реализации, та же публичная API). Curve/Image/Noise/SDF nodes, range analysis, FastNoise2, Expression node, bytecode VM — отложены |
| 2026-07-05 | Фаза 4: graph extensions + Cubes/Blocky mesher adapters | ✅ total 610 unit + 5 e2e. (1) `generators::graph` расширен 16 узлами: SDF (Plane/Box/Sphere/Torus/Union/Subtract/SmoothUnion/SmoothSubtract), Curve (baked lookup), Noise2D/3D (fastnoise-lite через NoiseConfig), math (Floor/Fract/Pow/Mix/Clamp/Distance2D/3D/Normalize3D). Все используют существующие `math::sdf` функции + `simple::Curve`/`NoiseConfig`. (2) `CubesMesher` adapter (VoxelMesher trait) — оборачивает greedy/simple cubes free functions, palette + opaque/transparent material split. (3) `BlockyMesher` adapter — оборачивает `blocky::mesher::generate_mesh`, shared `Arc<BakedLibrary>`, AO. Трио mesher adapters (Transvoxel/Cubes/Blocky) полностью готовы. 17 тестов (9 graph extensions + 4 CubesMesher + 3 BlockyMesher + empty-library edge) |
| 2026-07-06 | H1 full regular-mesh parity closed on macOS | ✅ C++ `transvoxel_sphere_16/32` goldens committed (`godot_voxel-cpp`): sphere_16 888 verts / 3912 idx, sphere_32 3696 verts / 18600 idx. Rust `transvoxel_parity` now passes against C++ goldens with exact structural fields and float tolerance. Root cause: C++ early-out uses raw SDF comparison, while case/interpolation use `sdf_as_float`; Rust now mirrors that split. `build_mesh.sh` fixed for macOS (`./build_mesh.sh`, BSD-sed-safe) |
| 2026-07-06 | Фаза 4: `VoxelEngine` foundation | ✅ total 631 unit + 10 integration. Engine-agnostic subset of `engine/voxel_engine.*`: generational `VolumeId`/`ViewerId`, volume/viewer registry, viewer position/distances/visual/collision/data-notification/network-peer metadata, `sync_viewers_task_priority_data` → shared `PriorityViewersData` (`highest_view_distance = max_distance * 2`, computed in `f32` to avoid `u32` overflow) and minimal `process()` sync. Task runners/dequeue loop/GPU/main-thread queues остаются следующим engine slice |
| 2026-07-06 | Фаза 4: `VoxelEngine` task loop | ✅ total 635 unit + 10 integration. `VoxelEngine` now owns `ThreadedTaskRunner`, exposes Rust-owned `push_async_task(s)` / `push_async_io_task(s)` wrappers, `wait_for_all_tasks`, `wait_and_clear_all_tasks`, thread-count controls, and `process()` drains completed tasks, enqueues follow-ups, applies `ThreadedTask::apply_result`, then syncs shared viewer priority data. Async-IO uses serial runner mode. Main-thread time-spread/progressive queues, GPU queue, file locker, stats/profiling and volume callback dispatch remain deferred |
| 2026-07-06 | Audit wave 1A: generator mutex removal | ✅ total 635 unit + 10 integration. `VoxelGenerator::generate_block`/`generate_single` now take `&self`; `SharedVoxelGenerator = Arc<dyn VoxelGenerator>`; external generator mutex removed from `VoxelData`, `MeshingDependency`, `MeshBlockTask`, `LoadBlockForTerrainTask`; `GraphGenerator` keeps synchronization local to its scratch. Remaining audit concurrency work after this step: `VoxelMesher::build(&self)` + scratch ownership, `VoxelData` per-LOD locks/`SpatialLock3D` integration, and data-lock ordering rule |
| 2026-07-06 | Audit wave 1B: mesher mutex removal | ✅ total 635 unit + 10 integration. `VoxelMesher::build` now takes `&self`; `SharedVoxelMesher = Arc<dyn VoxelMesher>`; external mesher mutex removed from `MeshingDependency` and `MeshBlockTask`; `TransvoxelMesher` moved its reuse `Cache` to thread-local scratch instead of serializing shared builds through an internal mutex. Remaining audit concurrency work: `VoxelData` per-LOD locks/`SpatialLock3D` integration, data-lock ordering rule, and perf reuse for `MeshArrays`/`MesherOutput` |
| 2026-07-06 | Audit wave 1C: `.vox` negative model-size guard | ✅ total 636 unit + 10 integration. `format::vox` now rejects `SIZE` dimensions outside `0..=MAX_MODEL_SIZE`, including `0xFFFFFFFF`/`-1`, before model allocation. Added regression test `parse_rejects_negative_model_size` |
| 2026-07-06 | Audit wave 1D: graph uniform-channel compression | ✅ total 637 unit + 10 integration. `GraphGenerator` now calls `VoxelBuffer::compress_uniform_channels()` after generation, matching C++ post-pass behavior and `HeightmapNoise`. Added regression test `generate_block_compresses_uniform_sdf_output` |
| 2026-07-06 | Audit wave 1E: Transvoxel uniform fast-path | ✅ total 638 unit + 10 integration. `TransvoxelMesher` now skips the full transvoxel O(n³) sampler path when the SDF channel is uniform, while preserving Rust's current one-empty-surface contract. Added regression test `transvoxel_mesher_fast_paths_uniform_sdf_without_sampling` |
| 2026-07-06 | Audit wave 1F: HeightmapNoise shared curve | ✅ total 639 unit + 10 integration. `HeightmapNoise::curve` now stores `Option<Arc<Curve>>`; `set_curve` wraps owned curves and `set_curve_arc` supports shared curve storage. Added regression test `heightmap_noise_curve_can_be_arc_shared` |
| 2026-07-06 | Audit wave 1G: RegionFile deferred header write | ✅ total 640 unit + 10 integration. `RegionFile` now tracks `header_dirty`: `save_block` marks the LUT dirty and `flush()`/`close()`/`Drop` persist the header once. Added regression test `save_block_defers_header_rewrite_until_flush` |
| 2026-07-06 | Audit wave 1H: data-lock ordering rule | ✅ total 645 unit + 10 integration. `LoadBlockForTerrainTask` snapshots stream/generator settings under `VoxelData` lock and performs stream/generator work after drop; `MeshBlockTask` queues missing gather regions under lock, fills them outside the critical section, then calls the mesher after lock release. Added four `try_lock()` guard tests covering generator/mesher/stream callbacks plus a deterministic shared-mesher overlap test |
| 2026-07-07 | Audit fix D3: VoxelBuffer create depth preservation | ✅ total 646 unit + 10 integration. `VoxelBuffer::create()` now preserves current per-channel depths when no explicit `VoxelFormat` is applied and resets channels to uniform defaults for those depths. `VoxelFormat::configure_buffer()` remains the explicit format application path. Added regression test `create_preserves_existing_channel_depths` |
| 2026-07-07 | Audit fix D6: safe blocky cutout bake | ✅ total 647 unit + 10 integration. `generate_library_cutout_sides` no longer uses raw-pointer aliasing; it computes cutout surfaces on a local `BakedModel` copy under shared library borrow and moves `cutout_side_surfaces` back. Added safety regression test `bake_module_uses_safe_cutout_driver` |
| 2026-07-07 | Audit fix D4: storage hot-path write helpers | ✅ total 649 unit + 10 integration. `VoxelBuffer`/`VoxelDataMap` hot accessors are now inline; `VoxelBuffer::fill_area` writes by row base; `downscale_to` and masked paste use safe depth-hoisted destination write helpers. Added structural guards `voxel_buffer_hot_paths_use_depth_hoisted_helpers` and `masked_paste_uses_depth_hoisted_destination_writes` |
| 2026-07-07 | Audit wave 2A: real SpatialLock3D | ✅ total 650 unit + 10 integration. `thread::SpatialLock3D` now tracks `(BoxBounds3i, mode)` entries behind `Mutex<Vec<_>> + Condvar`: overlapping reads coexist, overlapping writes block, disjoint writes proceed. Added tests `spatial_lock_3d_respects_overlap_and_mode` and `spatial_lock_3d_blocking_write_waits_for_overlapping_read` |

### Где остановились (для возобновления)

**Phase 0 — GO.** H1 regular-mesh parity и H2 performance проверены C++ harness'ем
без godot-cpp (stub-tree approach) и проходят: C++ goldens committed, Rust
`transvoxel_parity` воспроизводит их с точными structural fields и float tolerance.
Фаза 1
(`util/*`) — полностью портирована (191 тест).
Фаза 2 desktop-half — закрыт: `voxel-gdext` грузится в Godot 4.7, класс
`VoxelRustHello` виден в GDScript, достигает `voxel_core::VERSION` через FFI.
**Фаза 2 mobile-half — `.so` собран** (aarch64 + x86_64-android через NDK r29).
**Фаза 3 (compute-слой) — ЗАВЕРШЕНА.** Все engine-agnostic компоненты
портированы и повторно проверены audit pass'ом (439 unit тестов).
**Фаза 4 — storage/streaming + meshing pipeline + single-LOD paging terrain + VoxelEngine foundation/task loop работают headlessly (650 unit + 10 integration тестов).**
Audit wave 1A after the 2026-07-06 audit removed the outer generator mutex:
`VoxelGenerator` is shared via `Arc<dyn VoxelGenerator>` and called through `&self`.
Audit wave 1B removed the outer mesher mutex:
`VoxelMesher` is shared via `Arc<dyn VoxelMesher>` and called through `&self`;
`TransvoxelMesher` uses thread-local `Cache` scratch.
Audit wave 1H closed the current data-lock ordering rule: stream, generator and
mesher callbacks are not invoked while holding the outer `VoxelData` mutex.
End-to-end: generator → VoxelData → MeshBlockTask → TransvoxelMesher → MesherOutput.
Paging: VoxelTerrainCore orchestrates viewers → loads → meshing → outputs → unload.

**H1 (pass):** C++ и Rust генерируют одинаковый regular mesh для committed
goldens: sphere_16 = 888 verts / 3912 idx, sphere_32 = 3696 verts / 18600 idx.
Structural fields exact; float arrays compare with tolerance for C++ `%.8g`
JSON formatting/codegen drift. Root cause старого divergence: C++ делает
empty-cell early-out по raw SDF, а case/interpolation/normals — после
`sdf_as_float`; Rust теперь зеркалит этот split.
**H2 (pass):** Rust ~1.5× быстрее C++ (28.5µs/143Melem/s vs 44.1µs/93Mvoxels/s).
Полный разбор — в `rust/cpp-baseline/README.md` и `REPORT.md`.

**Открытые пункты:**
1. **Фаза 2 on-device** — загрузить `libvoxel_gdext.so` в Godot Android export
   template (нужен custom template `platform=android` + SDK + устройство/эмулятор).
   `.so` собирается локально через `rust/scripts/android-build.sh`; упаковка в APK
   и проверка на устройстве — вне данного окружения.
2. **Фаза 4 — что осталось (после single-LOD paging terrain):**
   - **Multi-LOD paging (VoxelLodTerrain)** — `VoxelLodTerrainUpdateData` +
     threaded update task (clipbox/octree стратегии). Single-LOD paging
     (`VoxelTerrainCore`) готов; multi-LOD — отдельный orchestrator.
   - **`VoxelEngine` remaining subset** — main-thread time-spread/progressive
     queues, GPU queue, file locker, stats/profiling and volume callback
     dispatch. Volume/viewer registry, shared priority viewer sync and threaded
     task drain loop are done.
   - **`generators::graph` runtime** — AST-walker + SDF/Curve/Noise/math nodes
     готовы; Image node, FastNoise2, range analysis, Expression node и bytecode
     VM — отложены.
   - **Mesher adapters** — Transvoxel/Cubes/Blocky wrappers готовы. Cubes/Blocky
     явно report `supports_lod=false`; Transvoxel сейчас regular path, transition
     mesh для variable LOD остаётся отдельным пунктом.
   - **`VoxelDataGrid`** — terrain meshing query helper (оптимизация; текущий
     MeshBlockTask обходит это через прямой `voxel_data.get_block` lookup).
   - **Concurrency audit follow-ups** — `VoxelData` per-LOD `RwLock` +
     real `SpatialLock3D` integration, plus stress/ThreadSanitizer coverage for the threaded
     edit/load/mesh path.
   - **ThreadSanitizer end-to-end** — когда `VoxelEngine` + real threading land.
   - **Godot binding (Phase 5)** — Node3D wrappers для VoxelTerrainCore +
     RenderingServer mesh upload + EditorPlugin.

### Фаза 1 (в работе)

Чистое ядро портится инкрементально, каждый модуль — отдельный коммит, clippy/fmt чист.

| Модуль | C++ источник | Статус |
|---|---|---|
| `math::box3i` | `util/math/box3i.h` | ✅ +13 тестов |
| `math::vector2` | `util/math/vector2{t,f,i}.h` | ✅ +8 тестов |
| `math::box2i` | `util/math/box2i.h` | ✅ +10 тестов (вынесен общий `funcs::clip_range`) |
| `containers` | `util/containers/{span,fixed_array,container_funcs}.h` | ✅ +8 тестов (Span→slice, FixedArray→`[T;N]`, Vec; алгоритмы shift_up/unordered_remove*/find_duplicate/is_uniform) |
| `math::sdf` | `util/math/sdf.h` (скалярная часть) | ✅ +5 тестов (box/sphere/plane/torus/CSG/round_cone; interval-перегрузки отложены с `interval.h`) |
| `math::color` | `util/math/color{,8}.h` | ✅ +6 тестов (Color rgba + lerp, Color8 + packed u8/u16/u32 конверсии) |
| `math::box3f` | `util/math/box3f.h` | ✅ +2 теста (float min/max bounds, contains, distance_squared) |
| `math::quaternion` | `util/math/quaternionf.h` | ✅ +2 теста (length/normalize, identity default) |
| `math::interval` | `util/math/interval.h` | ✅ +10 тестов (IntervalT<T> f32-инстанциация: все операции + интервальная математика min/max/sqrt/abs/clamp/lerp/sin/atan/atan2/floor/round/snapped/wrapf/smoothstep/squared/cubed/polynomial/get_length/pow; закрывает отложенные interval-SDF) |
| `math::basis3f` + `transform3f` | `util/math/{basis3f,transform3f}.h` | ✅ +7 тестов (3×3 rotation matrix + affine transform; +Index/IndexMut на Vector3T) |
| `math::conv` | `util/math/conv.h` (ZN-часть) | ✅ +2 теста (vec3i↔vec3f, floor/round/ceil_to_int) |
| `math::triangle` | `util/math/triangle.h` | ✅ +7 тестов (point-in-triangle, area/degenerate, barycentric round-trip, random-bary, ray–triangle f32+f64, baked-intersect-for-fixed-direction) |
| `math::box2f` | `util/math/box2f.h` | ✅ +4 теста (float 2D min/max box: from_min_size/intersects/clip/difference→Vec) |
| `math::box_bounds` | `util/math/box_bounds_{2i,3i}.h` | ✅ +5 тестов (min/exclusive-max int boxes: ctors, intersects, contains half-open, size) |
| `math::vector3i16` | `util/math/vector3i16.h` | ✅ +4 теста (`Vector3T<i16>` alias + i16 ops instance; `pack_hash` воспроизводит C++ sign-extension через int→uint64) |
| `hash` | `util/hash_funcs.h` | ✅ +9 тестов (djb2 32/64-bit combiner, MurmurHash3 one-shot, fmix32 finalizer — переносит Godot-core хеши в `crate::hash`) |
| `math::vector3i` utils | `util/math/vector3i.h` | ✅ +8 тестов (Vector3iUtil: create/sort_min_max/volume_u64/zxy+zyx index + inverse/distances/dot/abs/min/max/clamp/floordiv/ceildiv/wrap; 90° rotations + rotate_90/rotate_90_slice; Shl/Shr/BitAnd/Rem trait impls; Hash через djb2-цепочку Vector3iHasher; Eq на Vector3T для integer-алиасов) |
| `math::ortho_basis` | `util/math/ortho_basis.{h,cpp}` | ✅ +11 тестов (24-элементная lookup-таблица 90°-базисов + OrthoRotationId enum + name-таблица, позиционно связаны; from_axis_turns/transpose/invert/xform/composition; таблица верифицирована биективным round-trip) |
| `math::vector4` | `util/math/vector4{t,f}.h` | ✅ +5 тестов (Vector4T<T>: add/componentwise+scalar mul, Index; Vector4f math length_squared/normalized; исправлен upstream-баг `v.w+v.w`→`v.w*v.w` с заметкой в модуле) |
| `string` | `util/string/{conv,format}.{h,cpp}` | ✅ +14 тестов (conv: int32/int64 base10→buffer, float32/64 %g→buffer, string→int32 prefix-parse, константы размеров буферов; format: runtime `{}`-подстановка + dev hex-dump). **Skip:** `std_string`/`std_stringstream`/`fwd_std_string` (нативные Rust `String`/`str`). **Defer to Phase 3:** `expression_parser` (единственный потребитель — `generators/graph`) |
| `memory` | `util/memory/{memory,std_allocator}.h` | ✅ документирующий модуль (таблица C++→Rust: `ZN_ALLOC`/`UniquePtr`/`StdDefaultAllocator` → глобальный аллокатор + `Box`/`Vec`); debug-счётчики аллокаций за feature-флагом `alloc-counters` (+2 feature-gated теста) |
| `io::serialization` | `util/io/serialization.h` | ✅ +7 тестов (Endianness enum + platform-detect; MemoryWriter over `ByteSink` trait для `Vec<u8>` и fixed `ExistingBuffer`; MemoryReader get_8/16/32/64/float/buffer; float через to_bits/from_bits; round-trip big/little + bounds) |
| `io::text_writer` | `util/io/text_writer.{h,cpp}` + `std_string_text_writer.h` | ✅ +4 тестов (`TextWriter` trait с `drain`-sink + default `put_*`/`write_i64`/`f32`/`f64`/`bool`; `StringTextWriter` с `core::fmt::Write` для `write!`; методы prefixed `put_` чтобы не конфликтовать с `fmt::Write`) |
| `io::log` | `util/io/log.{h,cpp}` | ✅ +4 теста (глобальный verbose-флаг atomic + `print_line`/`print_warning`/`print_error`/`print_verbose`/`flush` через `eprintln!`/`println!`; voxel-gdext может переопределить для Godot-логгера) |
| `testing` | `util/testing/{test_directory,test_options}.h` | ✅ +7 тестов (`TestDirectory`: RAII temp-dir с recursive-drop-on-drop + `leak()`; `TestOptions`: include/exclude фильтры имён тестов `can_run`/`can_run_print`). `test_macros.h` → нативные `assert!`/`panic!` (документировано) |

**Фаза 1 (util/*) ЗАВЕРШЕНА.** `util/{math,string,memory,io,testing}` — все портированы.
**Отложено:** ~~`expression_parser` → Фаза 3~~ (теперь портирован — `string::expression_parser`, закрытие Фазы 1);
`file_locker` → Фаза 4 (следующий потребитель уже портированного `thread` layer).

### Фаза 3 (в работе)

Compute-слой. Каждый модуль — отдельный коммит, clippy/fmt чист.

| Модуль | C++ источник | Статус |
|---|---|---|
| `storage::voxel_memory_pool` | `storage/voxel_memory_pool.{h,cpp}` | ✅ +7 тестов (power-of-two block pool: 21 bucket до 1MiB, thread-safe recycle через Mutex<Vec> + atomics; идиоматичный Rust — owned Vec вместо raw pointers) |
| `storage::funcs` | `storage/funcs.{h,cpp}` | ✅ +9 тестов (copy_3d_region_zxy, fill_3d_region_zxy, transform_3d_array_zxy через OrthoBasis, snorm s8/s16↔float квантизация) |
| `storage::voxel_buffer` | `storage/voxel_buffer.{h,cpp}` | ✅ +16 тестов (полный multi-channel dense store: 8 каналов, depth 8/16/32/64-bit, UNIFORM/NONE компрессия, Default+Pool аллокаторы, get/set voxel raw+float, fill/fill_area, compress_uniform_channels, copy_channel_from, create-depth preservation, depth-hoisted write-area helper for downscale/masked writes, Drop возвращает пулу) |
| `storage::voxel_format` | `storage/voxel_format.{h,cpp}` | ✅ +5 тестов (per-channel depth descriptor + supported-depth ranges + default raw values) |
| `format::vox` | `streams/vox/vox_data.{h,cpp}` | ✅ +23 теста (MagicaVoxel `.vox` парсер: header SIZE/XYZI/RGBA/nTRN/nGRP/nSHP/LAYR/MATL чанки, scene-graph валидация, rotation-byte→Basis3f decode c fallback на identity для out-of-spec байт, default palette parity с C++ `g_default_palette`, `magica_to_opengl` axis swap). `Node` → идиоматичный Rust enum вместо C++ inheritance; `FileAccess` → `&[u8]` cursor с `Result<_, VoxError>`. Godot-shim `vox_loader.cpp` отложен до binding-слоя) |
| `io::serialization` (расширение) | `util/io/serialization.h` (MemoryReader) | ✅ +fallible API: `try_get_8/16/32/float` + `try_take` возвращают `Option` (без panic на EOF) и `set_endianness` для on-the-fly переключения byte order. Нужно для `instance_data` (чтение из untrusted-источников) и legacy v0 big-endian форматов |
| `streams::instance_data` | `streams/instance_data.{h,cpp}` | ✅ +13 тестов (lossy-compressed per-block instance transforms `FORMAT_SIMPLE_11B_V1`: position→3×u16, scale→u8, rotation→4×u8 quaternion; serialize/deserialize с v0 big-endian backcompat через `set_endianness`, trailing magic `0x900df00d`, scale-range clamp; `DeserializeError` enum вместо bool; round-trip тесты с quantization tolerance) |
| `streams::compressed_data` | `streams/compressed_data.{h,cpp}` | ✅ +12 тестов (LZ4/ZSTD compression envelope: NONE/LZ4/LZ4_BE(legacy big-endian)/ZSTD; LZ4 через **`lz4_flex`** (pure Rust, без C — важно для Android/WASM), ZSTD через optional `zstd` feature; `Compression` enum c wire-format discriminants, `Error` enum, round-trip для compressive/incompressible/empty payloads, byte-order проверка LZ4_BE vs LZ4, error paths). **Первая runtime-зависимость** voxel-core |
| `io::serialization` (расширение 2) | `util/io/serialization.h` (MemoryReader) | ✅ `try_get_64` добавлен (завершает fallible-семейство try_get_8/16/32/64/float + try_take) — нужен для `block_serializer` (UNIFORM-каналы depth 64-bit) |
| `storage::voxel_buffer` (расширение) | `storage/voxel_buffer.h` | ✅ `set_channel_depth` — setter для depth канала (нужен десериализатору; контракт: только на свежем uniform-канале, как в C++) |
| `streams::block_serializer` | `streams/voxel_block_serializer.{h,cpp}` | ✅ +11 тестов (`VoxelBuffer`↔bytes v4-формат: version + 3×u16 size + 8 каналов (fmt byte = compression\|depth<<4, raw/UNIFORM данные) + trailing magic `0x900df00d`; `serialize_and_compress`/`decompress_and_deserialize` обёртки над `compressed_data`; `Error` enum. **Metadata-секция отложена** — завязана на Godot Variant/custom-metadata factory (`storage/metadata/`, не портирован); v4 без metadata byte-совместим с C++ когда metadata пусто. v2/v3 legacy-миграция отложена по той же причине) |
| `generators::base` | `generators/voxel_generator.h` + `generators/simple/voxel_generator_heightmap.h` | ✅ `VoxelGenerator` trait (`generate_block`/`used_channels_mask`), `VoxelQueryData`/`GenResult` типы, `HeightmapParams` + переиспользуемая `generate_heightmap` generic-функция (closure `Fn(i32,i32)->f32`, range-remap, SDF/blocky ветки, early-exit выше/ниже heightmap). C++ Godot `Resource`+`RWLock`+GPU/cache API опущены — threading-layer в Фазе 4 |
| `generators::simple` | `generators/simple/{voxel_generator_waves,voxel_generator_flat,voxel_generator_noise}.{h,cpp}` | ✅ +21 тест (Waves +14, Noise +7). `Waves` — синусоидальный heightmap `0.5+0.25*(cos+sin)` через `generate_heightmap`; `Flat` — плоскость на высоте Y с SDF/blocky путями и two early-exit ветками; `Noise` — 3D SDF через **`fastnoise-lite`** (pure Rust, вторая runtime-зависимость), свой per-voxel loop `(noise_3d + bias) * noise_period` (не heightmap), early-exit с sentinels ±100, blocky/SDF каналы. Тесты: height-функция, bounded range, pattern offset/size, SDF gradient + iso_scale, blocky fill, early-exit, deterministic-same-seed, sign-change-in-slab |
| `meshers::cubes` | `meshers/cubes/{voxel_color_palette,voxel_mesher_cubes}.{h,cpp}` | ✅ +12 тестов (blocky mesher: `ColorPalette` `[Color8;256]` c default/serialise/u32 round-trip; greedy cube meshing — binary face-culling, ZXY indexing, face-axes+indices LUTs, alpha-class boundary detection, greedy rectangle merge along X then Y, opaque/transparent material split; `build_simple_cubes` non-greedy вариант. **Отложено**: atlased greedy mode (UV packing), `VoxelMesher` interface/Godot `Output` — завязаны на Godot `Array`/`Ref<Material>`/`Ref<Image>`) |
| `io::voxel_file` | `util/godot/classes/file_access.h` (subset) | ✅ +3 теста (`VoxelFile` trait: seek/position/len/read/write/set_len/flush + `StdVoxelFile` over `std::fs::File` + in-memory `MemoryFile` for tests. Stand-in for Godot `FileAccess`; `set_len` добавлен для sector compaction которого C++ не имеет — region files чисто truncated вместо stale trailing bytes) |
| `streams::region` | `streams/region/{region_file,file_utils}.{h,cpp}` | ✅ +22 теста (region-file archive format `.vxr`: `RegionFormat` (block_size_po2/region_size/channel_depths/sector_size/palette) с `validate`/`verify_block`/`header_size_v3`; `RegionBlockInfo` (24-bit sector_index + 8-bit sector_count packed u32); `RegionFile` — header save/load (VXR_ magic v3), deferred header flush, sector allocator (append/compact/`remove_sectors_from_block`), `load_block`/`save_block` через `block_serializer` + `compressed_data`. **Отложено**: forest wrapper (`VoxelStreamRegionFiles`, meta.vxrm JSON, LRU cache, lod-dir layout), v2→v3 legacy migration (needs `insert_bytes`), file locking) |
| `constants::cube_tables` | `constants/cube_tables.{h,cpp}` | ✅ +11 тестов (Side/Edge/Corner enums + LUTs: CORNER_POSITION, SIDE_NORMALS, SIDE_CORNERS, SIDE_EDGES, EDGE_CORNERS, OPPOSITE_SIDE, MOORE_NEIGHBORING_3D, ORDERED_MOORE_AREA_3D; `dir_to_side`. Фундамент для blocky/cubes meshers) |
| `meshers::blocky::baked_library` | `meshers/blocky/blocky_baked_library.h` | ✅ +8 тестов (plain-data model library: `BakedModel`/`BakedModelMesh`/`ModelSurface`/`SideSurface` с geometry arrays + side masks + pattern indices; `BakedLibrary` с `DynamicBitset` occlusion matrix; `BakedFluid`/`FluidSurface`; `Aabb` stand-in для Godot AABB. Godot Resource/editor слой → Фаза 5) |
| `meshers::blocky::bake` | `meshers/blocky/voxel_blocky_library_base.cpp` (bake pass) | ✅ +10 тестов (side-culling matrix generation: `SideBitmap` `[u64;4]` rasterization, `detect_single_quad`, `rasterize_triangle_barycentric`, `generate_side_culling_matrix` — rasterizes side geometry → deduplicates patterns → builds occlusion matrix + `full_sides_mask`/`empty_sides_mask`/`side_pattern_indices`/`contributes_to_ao`; cutout-surface baking without raw-pointer aliasing. `bake_library` entry point) |
| `meshers::blocky::mesher` | `meshers/blocky/voxel_mesher_blocky.{h,cpp}` (core) | ✅ +7 тестов (`generate_mesh<T>` core algorithm: neighbor-based face culling via `is_face_visible*`, baked geometry emission, 0fps-style corner ambient occlusion, cutout-surface lookup. `BlockyArrays` output struct. Godot `VoxelMesherBlocky` class/build()/`_bind_methods` → Фаза 5) |
| `meshers::blocky::lod_skirts` | `meshers/blocky/blocky_lod_skirts.h` | ✅ +6 тестов (`append_skirts`/`append_side_skirts` — LOD seam-skirt geometry для всех 6 сторон. `TintSampler` → `skirt_depth: f32` (tint integration отложен до Фазы 5)) |
| `meshers::blocky::shadow_occluders` | `meshers/blocky/blocky_shadow_occluders.{h,cpp}` | ✅ +12 тестов (`generate_shadow_occluders`/`generate_occluders_geometry`/`classify_chunk_occlusion_from_voxels` — shadow geometry из per-face box quads с точным winding per side; `ShadowOccluderArrays`; две bit-ordering конвенции reproduced + задокументированы) |
| `streams::stream_cache` | `streams/voxel_stream_cache.{h,cpp}` | ✅ +8 тестов (`BlockCache` — in-memory `(Vector3i, lod) → VoxelBuffer` cache через `HashMap`. RWLock опущен — single-threaded как весь Rust-порт; threading в Фазе 4) |
| `streams::stream_memory` | `streams/voxel_stream_memory.{h,cpp}` | ✅ +8 тестов (`MemoryStream` — "fake" in-memory stream для тестов: `save_block`/`load_block`/`SaveMode`/`LoadResult`; теперь реализует Phase 4 `VoxelStream` trait и защищает storage через `RwLock`) |
| `generators::simple::HeightmapNoise` | `generators/simple/voxel_generator_noise_2d.{h,cpp}` | ✅ +6 тестов (2D-noise heightmap через `generate_heightmap`: `NoiseConfig` (Clone-able seed/freq/type → rebuild FastNoiseLite per call), optional `Curve` remap `[0,1]→height` с linear interpolation, `compress_uniform_channels` post-generation. Godot `Ref<Noise>`/`Ref<Curve>`/signal handling → Фаза 5) |

**Фаза 3 (compute-слой) ЗАВЕРШЕНА.** Все engine-agnostic компоненты портированы:
`storage` (4 модуля), `format::vox`, `streams` (instance_data, compressed_data,
block_serializer, region, stream_cache, stream_memory), `generators` (base,
Waves, Flat, Noise, HeightmapNoise), `meshers` (transvoxel, cubes, blocky с
полным bake+mesher+skirts+shadow), `constants::cube_tables`, `io` extensions.

**Отложено в Фазу 4** (threading/terrain): threaded tasks (`*_task.cpp`),
`VoxelData`/streaming dependency.

**Отложено в Фазу 5** (Godot binding): все `Resource`/`GDCLASS`/`Ref<Material>`/
`Ref<Mesh>`/editor слои (`VoxelMesherBlocky`, `VoxelBlockyLibraryBase`,
`VoxelBlockyModel*`, `voxel_block_serializer_gd`), atlased cubes mode (UV
packing + `Ref<Image>`), `generators::graph` (нужен `expression_parser`),
metadata-секция block_serializer (Godot Variant codec), v2/v3 legacy migration.

### Фаза 4 (в работе) — план порта

Threading + terrain — самый сложный этап. Порядок по зависимостям:

| # | Модуль | C++ источник | Зависимости | Заметки |
|---|---|---|---|---|
| 4.1 | `thread::{mutex,rw_lock,semaphore,spatial_lock_3d}` | `util/thread/{mutex,rw_lock,semaphore,spatial_lock_3d}.{h,cpp}` | `std::sync` | ✅ recursive `Mutex`, `BinaryMutex`, `RwLock`, `Semaphore`, real `SpatialLock3D` overlap-aware region lock; открывает file_locker + VoxelStream + A3 per-region locking |
| 4.2a | `tasks::task_priority` / `tasks::cancellation_token` | `util/tasks/{task_priority,cancellation_token}.h` | atomics | ✅ packed 4-band priority + shared cancel flag |
| 4.2b | `tasks::threaded_task` / `tasks::threaded_task_runner` | `util/tasks/*` | thread, `IThreadedTask` | ✅ Owned `Box<dyn ThreadedTask>` runner: priority polling, serial gate, postponed queue, cancellation skip, completed drain + follow-up enqueue helper, shutdown |
| 4.3 | `io::file_locker` | `util/io/file_locker.h` | mutex | ✅ RAII per-path read/write guards; entries persist like C++ map |
| 4.4 | `streams::voxel_stream` (base trait) | `streams/voxel_stream.{h,cpp}` | RWLock, VoxelBuffer | ✅ `VoxelStream: Send + Sync`, `LoadResult`/`SaveMode`, batch defaults; `MemoryStream` impl |
| 4.4a | `engine::{priority_dependency,streaming_dependency}` / `tasks::async_dependency_tracker` | `engine/*dependency*`, `util/tasks/async_dependency_tracker.*` | tasks, VoxelStream, atomics/locks | ✅ mutable viewer priority/drop-distance evaluation, stream invalidation handle, race-free async countdown with next-task handoff |
| 4.5a | `streams::save_block_data_task` | `streams/save_block_data_task.{h,cpp}` | tasks, VoxelStream, VoxelBuffer | ✅ voxel-only save task: priority, tracker abort/complete, last-task flush, follow-up task handoff |
| 4.5b | `streams::{block_data_output,load_block_data_task}` | `streams/load_block_data_task.{h,cpp}`, `VoxelEngine::BlockDataOutput` | tasks, VoxelStream, VoxelFormat | ✅ stream I/O half with explicit format/block size, output kinds for loaded/not found/needs generation; VoxelData callbacks later |
| 4.5c | `streams` generator handoff | `GenerateBlockTask`, `VoxelGenerator` | load task, generators | ✅ Minimal handoff hook: cache miss can create a generator follow-up task through `BlockGenerationTaskFactory`; concrete `GenerateBlockTask` + `VoxelData` integration continues in 4.6 |
| 4.6a | `storage::{voxel_data_block,voxel_data_map}` | `storage/voxel_data_block.*`, `storage/voxel_data_map.*` | VoxelBuffer, VoxelFormat, Box3i | ✅ Sparse block storage base: optional voxel payloads, edit/modified/LOD flags, block/local coordinate conversion including negative coords, default reads, block overwrite/removal, area-loaded checks |
| 4.6b | `storage::voxel_data_map` copy/paste | `storage/voxel_data_map.cpp` (`copy`, `paste`) | VoxelDataMap, VoxelBuffer | ✅ Basic channel-mask copy/paste across sparse blocks; `create_new_blocks=false` skips missing/empty blocks. Metadata, source/destination masks and generator callback copy remain pending |
| 4.6c | `storage::voxel_data_map` source-mask paste | `storage/voxel_data_map.cpp` (`paste_masked`) | VoxelDataMap, VoxelBuffer | ✅ Source-mask paste skips voxels whose source mask channel equals the mask value. Destination writable masks, metadata and generator callback copy remain pending |
| 4.6d | `storage::voxel_data_map` destination-mask paste | `storage/voxel_data_map.cpp` (`paste_masked`) | VoxelDataMap, VoxelBuffer | ✅ Destination writable-value mask for source-masked paste, matching negative-coordinate C++ parity test. Metadata and generator callback copy remain pending |
| 4.6e | `storage::{voxel_buffer,voxel_data_map}` region copy/paste | `storage/voxel_buffer.cpp` (`copy_channel_from` area overload), `storage/voxel_data_map.cpp` (`copy`, `paste`) | `copy_3d_region_zxy`, VoxelBuffer, VoxelDataMap | ✅ Area copy helper with uniform/materialized channel handling; unmasked `VoxelDataMap::copy`/`paste` now iterate block regions instead of per voxel. Masked predicate paste remains per-voxel pending a dedicated helper |
| 4.6f | `storage::voxel_data` aggregate MVP | `storage/voxel_data.{h,cpp}` | VoxelDataMap, VoxelDataBlock, VoxelFormat | ✅ First synchronous VoxelData aggregate: LOD maps, bounds/format/full-load/streaming flags, block/voxel access, block insertion, LOD0 modification flags. Generator pre-generation, LOD downscaling, save/unload consumption and streaming task integration remain pending |
| 4.6g | `storage::voxel_data` generation/save hooks | `storage/voxel_data.cpp` (`pre_generate_box`, `consume_*modifications`, `unload_blocks`) | VoxelData, VoxelGenerator, VoxelBuffer | ✅ Sync storage subset: generator-backed `pre_generate_box` (non-streaming creates missing blocks; streaming fills existing empty blocks), `BlockToSave`, modification consumption, and unload-with-save handoff. Stream delete persistence, refcounted view/unview and terrain scheduler integration remain pending |
| 4.6h | `storage::voxel_buffer::downscale_to` | `storage/voxel_buffer.cpp` (`downscale_to`) | VoxelBuffer | ✅ Nearest-neighbor 2:1 mip-map kernel: ZXY loop order, uniform-channel fast path, source/dst region clamping. Foundation for `update_lods` |
| 4.6i | `storage::voxel_data::update_lods` | `storage/voxel_data.cpp` (`update_lods`) | VoxelData, downscale_to, generator | ✅ LOD cascade: LOD0 needs_lodding clear + pairwise up-pass (src→dst octant via `rel*half_bs`), generates missing dst blocks in non-streaming mode, marks dst modified, enqueues next LOD. `BlockLocation` out-param |
| 4.6j | `storage::voxel_data` viewers + view/unview | `storage/voxel_data_block.h` (`RefCount`), `storage/voxel_data.cpp` (`view_area`, `unview_area`) | VoxelDataBlock, VoxelData | ✅ `Viewers` refcount (saturating add/remove) on VoxelDataBlock; `view_area` increments + reports found/missing; `unview_area` decrements + removes zero-count blocks, returns modified for save |
| 4.6k | `storage::voxel_data` generator/stream ownership | `storage/voxel_data.{h,cpp}` (`set_generator`, `set_stream`) | VoxelData, VoxelGenerator, VoxelStream | ✅ `SharedVoxelGenerator = Arc<dyn VoxelGenerator>`, `SharedVoxelStream = Arc<dyn VoxelStream>`; `with_generator` helper; `VoxelGenerator: Send + Sync` and `generate_block(&self)`. `get_voxel` generator fallback (`generate_single`) is done |
| 4.6l | `storage::voxel_data` copy/paste + area queries | `storage/voxel_data.cpp` (`copy`, `paste`, `paste_masked*`, `is_area_loaded`, `has_all_blocks_in_area`, `get_missing_blocks`, `get_blocks_with_voxel_data`) | VoxelData, VoxelDataMap | ✅ VoxelData-level copy (с generator fallback для missing blocks), paste/paste_masked/paste_masked_with_destination_mask делегируют в LOD0 map; area queries: is_area_loaded (streaming-aware), has_all_blocks_in_area, get_missing_blocks, get_blocks_with_voxel_data (ZXY grid) |
| 4.6m | `paste_masked` per-block + O(1) writability lookup | `storage/voxel_data_map.cpp` (`paste_masked`) | VoxelDataMap | ✅ Audit-driven optimization: per-block iteration (1 hashmap lookup/block вместо per-voxel), `WritabilityLookup` dense `Vec<bool>` для u16-fitting values (как C++ `DynamicBitset`), linear fallback для крупных значений, masked destination writes through `VoxelBuffer::read_write_area*` depth-hoisted helpers |
| 4.6n | `ThreadedTaskRunner` priority throttle | `util/tasks/threaded_task_runner.cpp` (`_priority_update_period_ms`) | tasks | ✅ Audit-driven optimization: `priority_update_period` (default 32 ms как C++) + `last_priority_update`; worker вызывает `refresh_priorities_and_complete_cancelled` только когда окно прошло, не на каждом wake |
| 4.6 | `storage::voxel_data` | `storage/voxel_data.{h,cpp}` | VoxelDataMap, streams, generators, LOD | In progress: `get_voxel` generator fallback (`generate_single`), metadata, `VoxelDataGrid`, `try_set_block` with action_when_exists. Per-LOD RWLock + `SpatialLock3D` integration отложены (пока single-threaded через borrow checker) |
| 4.7 | `terrain::voxel_terrain` / `voxel_lod_terrain` | `terrain/*` | VoxelData, meshers, Node3D | VoxelTerrain node (без Godot binding — pure logic) |
| 4.8 | `generators::graph` (runtime) | `generators/graph/*` | `string::expression_parser` (Phase 1 deferred) | Graph-based procedural gen без редактора |
| 4.9 | Integration test | — | все выше | End-to-end: generator → VoxelData → mesher → mesh, ThreadSanitizer |

**GO-критерий Phase 4:** стриминг бесконечного terrain'а работает, нет race
conditions (проверка под ThreadSanitizer/loom).

**Ключевые риски Phase 4:**
- `Gd<T>` lifetime на gdext + threading — ownership-модель должна быть спроектирована рано
- VoxelData — сложный streaming grid с LOD, locking, eviction (самый большой C++ компонент)
- VoxelTerrain — Godot Node3D, завязан на `Engine::get_main_loop`/`RenderingServer`

### Команды для возобновления работы
```bash
git clone https://github.com/sandsaber/godot_voxel.git
cd godot_voxel && git checkout rust/pilot
cd rust
cargo test -p voxel-core       # 650 unit + 10 integration + 1 doc-test; 1 ignored diagnostic snapshot
cargo build -p voxel-gdext     # GDExtension .so (грузится в Godot 4.7)
cargo clippy --workspace --all-targets  # должен быть чистый
cargo bench                    # transvoxel benches (16³=143 / 32³=199 / 64³=249 Melem/s)
./cpp-baseline/build_mesh.sh   # C++ mesh harness (H1 parity + H2 perf baseline vs Rust)
./scripts/android-build.sh                  # Android aarch64 gdext .so (NDK r29; rust-lld workaround)
./scripts/android-build.sh --target x86_64-linux-android   # эмуляторный .so
./scripts/android-build.sh --core            # voxel-core staticlib (.a) — Phase 0 H4
```

### Ключевые находки сессии
- **ZXY memory layout** в C++ `VoxelBuffer` (`index = y + sy*(x + sx*z)`, Y innermost) —
  зафиксировано в `voxel_index` и `build_regular_mesh`.
- **LLVM skew** rustc 1.96.1 (LLVM 22) vs NDK r29 (LLVM 21): NDK `lld` не читает объекты rustc
  при линковке `.so` (`Unknown attribute kind 103`). Workaround — заставить NDK-clang линковать
  rust-`lld` через `-fuse-ld=lld` + symlink. Зафиксировано в `rust/scripts/android-build.sh`.
  `.a` (статический архив) этому не подвержен — работает даже без NDK.
- **voxel-core чистый Rust без FFI** → кросс-компилируется в `.a` под Android и Apple arm64
  прямо с Linux, без SDK (нужен только встроенный `llvm-ar`). SDK/NDK потребуются только на
  финальной линковке `.so`/`.dylib` в Фазе 2.

### Решение по внешним крейтам (после исследования экосистемы)
- `transvoxel` (Gnurfos, v2.0) — **НЕ используем напрямую**: статус experimental,
  другой формат вывода, нет parity с C++. Свой порт даёт byte-for-byte совместимость.
- `lz4_flex` (pure Rust, v0.11) — **принято для `streams::compressed_data`** ✅: fastest
  pure-Rust LZ4, byte-совместим с C-референсом на уровне блока, `#![no_std]`-совместим.
  Выбран вместо `lz4` (C bindings) ради чистой Android/WASM кросс-компиляции.
- `zstd` (v0.13, C bindings через `zstd-sys`) — **optional feature** в voxel-core: C++
  поддерживает ZSTD всегда, но для дефолтной мобильной сборки вынесен за флаг `zstd`.
  Без флага ZSTD-streams возвращают `Error::Unsupported`.
- `block-mesh-rs` (https://github.com/bonsairobo/block-mesh-rs) — **кандидат для будущей оптимизации block/cube mesher нагрузки**: оценить адаптацию `visible_block_faces` и `greedy_quads` после Phase 4 terrain integration. Не добавлять зависимость автоматически: сначала сравнить с текущими `meshers::{cubes,blocky}` по parity, материалам/палитрам, прозрачности и LOD skirts.
- `fastnoise-lite` (pure Rust) — **кандидат для замены FastNoise2** без C++ FFI.
- Rapier3d — **primary для физики** (Фаза 3-4), Avian — fallback.

---

## 8. Альтернативы (если Фаза 0 покажет NO-GO)

Если пилот покажет, что Rust+gdext не даёт нужной скорости/эргономики:

**План B — Гибрид FFI:** Rust-ядро как статическая библиотека, вызывается из
существующего C++ GDExtension через `cbindgen`. Получаешь безопасность Rust на
hot-path, не платишь за риски gdext. Сохраняешь весь C++ Godot-слой как есть.

**План C — Только GDExtension, без Rust:** Принять, что C++ GDExtension у Zylann
уже работает. Сфокусироваться на мобильной валидации существующей C++ версии
(NDK-сборка). Это самый дешёвый путь к мобильным билдам.

**План D — Отказаться от Web цели:** Если Web — единственный мотиватор,
перепроектировать threading под cooperative модель можно и в C++. Rust не обязателен.

---

## 9. Физика: Rapier (primary), Avian (fallback)

Текущий `godot_voxel` использует гибридную физику:
- Godot physics для general rigid bodies (через узлы terrain'а)
- Кастомную Minecraft-like коллизию (`VoxelBoxMover`, `edition/raycast.cpp`) для
  быстрых проверок против вокселей

**Решение:** мигрировать физику на нативный Rust physics engine.

| | **Rapier3d** (primary) | **Avian** (fallback) |
|---|---|---|
| Solver | Impulse-based | XPBD |
| Зависимости | Standalone, чистый Rust | **Привязан к Bevy ECS** ⚠️ |
| Зрелость | Очень зрелый, продакшн | Моложе, активно развивается |
| Determinism | ✅ | ✅ |
| Multithreading | ✅ Rayon | ✅ Bevy scheduler |

**Почему Rapier как primary:** standalone, embeddable, не тащит Bevy в `voxel-core`.
Avian рассматривается только если в будущем появится нативная Bevy-интеграция
в проекте.

Архитектурно физика ложится в `voxel-core/physics/`:
- `voxel_collider.rs` — динамический collider из voxel data
- `raycast.rs` — замена VoxelRaycaster
- `body_mover.rs` — замена VoxelBoxMover

Планируется к миграции в Фазе 3-4 после валидации основного стека.
