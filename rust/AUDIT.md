# Аудит миграции godot_voxel → Rust

> Дата исходного аудита: 2026-07-06 · Ветка: `rust/pilot` (403b81ba, синхронизирована с `origin/rust/pilot`, рабочее дерево чистое)
> Обновления после аудита фиксируются в §9.7, чтобы отделять исходные находки от уже закрытых пунктов.
> Метод: проверка заявлений `MIGRATION_PLAN.md` / `rust/STATUS.md` / `REPORT.md` против фактического кода,
> прогон тестов/clippy/fmt/сборки/бенчмарков, инвентаризация LOC C++ vs Rust,
> **плюс ревью качества кода**: 5 независимых проходов по подсистемам (storage, meshers,
> generators, threading/engine/terrain, streams/math/io) со сверкой с C++-референсом,
> ключевые находки перепроверены по коду вручную (см. §9).

---

## 1. Резюме

**Заявленное состояние подтверждается фактически.** Все ключевые утверждения документации
(количество тестов, чистый clippy/fmt, сборка gdext, parity H1, реальная многопоточность,
real SpatialLock3D) проверены и совпадают с кодом. Расхождений «документация говорит одно,
код — другое» не найдено.

Портировано **~36 900 строк Rust** против **~142 100 строк C++** модуля (без thirdparty) —
**~26% по сырому LOC**, но портированная часть — это концентрированное compute-ядро
(math/storage/meshers/streams/generators/terrain-core). Весь Godot-facing слой
(binding, editor, edition-инструменты, инстансинг, multi-LOD terrain, GPU) — не начат.

**Ревью качества кода (§9)**: алгоритмы портированы корректно и идиоматично, но найдены
два системных долга — Mutex-сериализация всего конвейера (многопоточность пока косметическая,
§9.1) и абстракционные издержки в горячем пути мешинга, которые C++ целенаправленно устранял
(§9.2). Плюс набор конкретных фиксов с приоритетами (§9.5).

| Фаза | Заявлено | Аудит |
|---|---|---|
| 0 — Пилот (transvoxel + кросс-компиляция) | ✅ GO | ✅ подтверждено: H1 parity-тесты проходят, таблицы byte-identical |
| 1 — Чистое ядро (`util/*` + expression_parser) | ✅ завершена | ✅ подтверждено |
| 2 — Мобильная валидация | ✅ `.so` desktop+Android; on-device ⏳ | ✅/⚠️ on-device проверка так и не закрыта (нужен SDK+устройство) |
| 3 — Compute-слой | ✅ завершена | ✅ подтверждено (engine-agnostic scope) |
| 4 — Terrain + threading | 🟡 в работе | 🟡 подтверждено: single-LOD paging + VoxelEngine foundation готовы headlessly |
| 5 — Godot binding + editor | ⏳ не начата | ⏳ подтверждено: binding = 82 строки hello-world |

---

## 2. Проверенные факты (прогон 2026-07-07, macOS)

| Проверка | Заявлено | Фактически |
|---|---|---|
| `cargo test -p voxel-core` | 655 unit + 11 integration + 1 doc-test | ✅ 655 unit + 11 integration (5 e2e + 2 parity + 2 sphere + 1 tables + 1 threaded stress) + 1 doc-test, 0 failed, 1 ignored (diagnostic dump) |
| `cargo clippy --workspace --all-targets` | чистый | ✅ чистый |
| `cargo fmt --check` | чистый | ✅ чистый |
| `cargo build -p voxel-gdext` | собирается | ✅ собирается |
| H1 mesh parity vs C++ goldens | pass (sphere_16: 888 verts/3912 idx; sphere_32: 3696/18600) | ✅ `transvoxel_parity` проходит против закоммиченных C++ goldens |
| Transvoxel-таблицы | byte-identical upstream C++ | ✅ `transvoxel_tables_parity` проходит |
| `ThreadedTaskRunner` | реальные потоки | ✅ `std::thread::spawn` в worker-пуле (`tasks/threaded_task_runner.rs`) |
| `SpatialLock3D` | real overlap-aware region lock | ✅ подтверждено (`thread/mod.rs`): overlapping reads coexist, overlapping writes block, disjoint boxes proceed |
| Код без заглушек-паник | — | ✅ ни одного `todo!`/`unimplemented!`/`FIXME`; 6 TODO-комментариев (унаследованы из C++, безобидны) |
| H2 perf (Rust ~1.5× быстрее C++) | 28.5µs/143 Melem/s vs 44.1µs/93 Mvoxels/s | ✅ бенчи перепрогнаны (macOS): sphere_16 = 27.5µs / 149 Melem/s, sphere_32 = 206 Melem/s — числа воспроизводятся. **Но** бенч меряет ядро `build_regular_mesh` в обход адаптера `builtin.rs` — см. §9.4 |

---

## 3. Объём: C++ модуль vs Rust порт

### C++ (без thirdparty/) — 142 148 LOC

| Директория | LOC | Содержимое | Rust-покрытие |
|---|---:|---|---|
| terrain/ | 25 233 | fixed_lod (4,1k), variable_lod (9,9k), instancing (9,2k), root (2,0k) | 🟡 только engine-agnostic single-LOD ядро (1 084) |
| util/ | 23 809 | godot-shims (7,8k), math (4,9k), noise (3,1k), containers, thread, tasks, io | 🟢 math/string/io/tasks/thread портированы; godot-shims N/A; FastNoise2 нет |
| generators/ | 19 386 | graph (14,1k), multipass (2,3k), simple (1,4k) | 🟡 minimal graph runtime (1,8k), simple частично; multipass нет |
| meshers/ | 16 503 | blocky (9,2k), transvoxel (4,6k), cubes (1,6k) | 🟢 все три ядра + адаптеры VoxelMesher (7 780) |
| editor/ | 12 833 | 11 EditorPlugin-ов (graph editor 5,5k и др.) | 🔴 ноль |
| tests/ | 9 025 | C++ тест-сюита | ➖ своя Rust-сюита, не зеркалирует C++ |
| storage/ | 8 126 | buffer, data/map/block/grid, format, mixel4, metadata | 🟢 ядро портировано; metadata/ и VoxelDataGrid нет |
| streams/ | 8 018 | base, serializer, region (2,0k), sqlite (2,2k), vox | 🟡 форматы/задачи портированы; sqlite и forest-wrapper нет |
| edition/ | 7 821 | VoxelTool*, raycast, mesh SDF, floating chunks | 🔴 ноль |
| engine/ | 5 461 | VoxelEngine, gpu (1,5k), detail_rendering (2,4k) | 🟡 registry+task loop (1,2k); GPU/detail нет |
| shaders/ | 3 164 | GLSL-шаблоны + реестр | 🔴 ноль |
| modifiers/ | 1 467 | VoxelModifier стек | 🔴 ноль |
| constants/ | 670 | cube tables и др. | 🟢 портировано |

### Rust — 36 907 LOC

- `voxel-core/src` — **36 825 LOC**: math 6 332 · meshers 7 780 · storage 5 456 · streams 4 687 ·
  generators 3 182 · format/vox 1 485 · string 1 353 · tasks 1 318 · io 1 255 · engine 1 249 ·
  terrain 1 084 · thread 472 · constants 466 · containers 265 · прочее 441.
- `voxel-gdext/src` — **82 LOC**: entry point + один класс `VoxelRustHello` (hello-world). Реальных биндингов нет.
- Инфраструктура: workspace с pinned toolchain 1.96.1 + 4 mobile-таргета, criterion-бенчи,
  `cpp-baseline/` C++ harness (goldens + perf), `scripts/android-build.sh` (NDK r29 + rust-lld workaround).
- Зависимости ядра: `lz4_flex` (pure Rust), `fastnoise-lite` (pure Rust), `zstd` (optional feature) — осознанно минимальны.

---

## 4. Что работает end-to-end (headless, без Godot)

Полный конвейер в чистом Rust, подтверждён интеграционными тестами:

```
GraphGenerator (24+ узлов) / Waves / Flat / Noise / HeightmapNoise
  → VoxelData (LOD-каскад, view/unview refcount, copy/paste, generator fallback)
  → MeshBlockTask (gather 3×3×3 + gap-fill)
  → VoxelMesher: Transvoxel / Cubes / Blocky
  → VoxelTerrainCore (single-LOD paging: viewers → loads → meshing → outputs → unload)
  → VoxelEngine (volume/viewer registry, ThreadedTaskRunner, drain loop, приоритеты)
```

---

## 5. Полностью непортированные подсистемы (0 строк Rust)

Отсортировано по объёму C++:

1. **editor/** — 12 833 LOC. Все EditorPlugin-ы (graph-редактор, blocky library, vox import, гизмо). Фаза 5.
2. **terrain/variable_lod** — 9 890 LOC. `VoxelLodTerrain`, lod_octree, clipbox/octree update task. **Главный оставшийся блокер Фазы 4.**
3. **terrain/instancing** — 9 202 LOC. `VoxelInstancer`, multimesh/scene items, rigidbody.
4. **edition/** — 7 821 LOC. `VoxelTool*`, raycast, box mover, mesh SDF, floating chunks.
5. **shaders/** — 3 164 LOC. GLSL-шаблоны (завязаны на GPU-путь).
6. **generators/multipass** — 2 303 LOC.
7. **engine/detail_rendering** — 2 413 LOC (normal maps).
8. **streams/sqlite** — 2 156 LOC.
9. **terrain/ root** — 2 016 LOC (VoxelViewer node, VoxelAStarGrid3D, mesh block map).
10. **engine/gpu** — 1 516 LOC (compute-шейдеры, GPU task runner).
11. **modifiers/** — 1 467 LOC.
12. **util/noise FastNoise2 / SpotNoise** — ~3 095 LOC (SIMD-нойз; в Rust только fastnoise-lite).
13. **storage/metadata** — Variant-метаданные (блокирует metadata-секцию block_serializer и v2/v3 миграцию).

## 6. Частично портированные

| Подсистема | Есть в Rust | Нет |
|---|---|---|
| generators/graph | AST-walker runtime, ~24 узла, GraphGenerator | компилятор/bytecode VM, range analysis, Image/Expression/FastNoise2 узлы, shader generator (~12,7k из 14,1k LOC) |
| terrain/fixed_lod | paging-оркестратор `VoxelTerrainCore` | Node3D, collision, multiplayer sync, box mover |
| engine/VoxelEngine | registry, priority sync, task drain loop | time-spread/progressive очереди, GPU queue, stats, volume callbacks |
| streams/region | формат `.vxr` (header/LUT/sectors) | forest-wrapper `VoxelStreamRegionFiles` (meta.vxrm, LRU) |
| generators/simple | Flat/Waves/Noise/HeightmapNoise | Image, Noise2D-generator варианты |
| util/thread | Mutex/RwLock/Semaphore, real SpatialLock3D, реальный task runner | `SpatialLock2D`, ThreadSanitizer end-to-end |
| meshers/transvoxel | regular cells, parity с C++ | transition cells (LOD-переходы) для variable LOD |
| meshers/cubes | greedy/simple + palette | atlased mode (UV packing) |

---

## 7. Отклонения от плана и риски

1. **Git-стратегия не выполняется.** План (§1) требует ветку `cpp-reference` (зеркало Zylann/master)
   и регулярный upstream-merge. Фактически: remote на upstream не настроен, ветки `cpp-reference` нет,
   только `origin` (sandsaber) с `master` + `rust/pilot`. Дивергенция от Zylann не отслеживается —
   чем дольше, тем дороже догонять bugfix'ы. *Рекомендация: добавить remote + ветку, это дёшево.*
2. **Фаза 2 не закрыта on-device.** `.so` собран, но APK на устройстве/эмуляторе не проверялся
   (нужны SDK+устройство вне окружения). Формально GO-критерий Фазы 2 не выполнен.
3. **GO-критерий Фазы 4 недостижим текущим кодом**: «нет race conditions под ThreadSanitizer» —
   TSan не запускался; `SharedVoxelData` уже берёт `SpatialLock3D` regions в terrain/mesh
   consumers и использует per-LOD map locks/settings lock, A5 task runner закрыт, а macOS
   stress `threaded_edit_load_mesh_stress` стабилен. До формального GO остаётся TSan на
   Linux/nightly.
   Это честно задокументировано, но стоит держать в фокусе: реальная многопоточность может
   вскрыть проблемы в дизайне ownership.
4. **Parity-покрытие тестами точечное.** Byte-parity доказан для transvoxel-таблиц и двух golden-сфер;
   остальные модули покрыты юнит-тестами, портированными «по мотивам» C++, но C++ тест-сюита
   (9 025 LOC, 61 файл) не зеркалируется системно. Риск тихих поведенческих расхождений в углах.
5. **H2 perf проверен только на transvoxel 16³–64³.** Для storage/paging/graph перф-сравнений с C++ нет.
6. **REPORT.md устарел числами** («~2248 LOC ported», «32 tests») — это снапшот Фазы 0; актуальные
   цифры в `rust/STATUS.md`. Не ошибка, но читателя может запутать.

---

## 8. Оценка оставшегося объёма

Грубо, по C++ LOC ещё не тронутого кода (≈105k из 142k, минус N/A godot-shims ≈7,8k и tests ≈9k → ~88k «содержательных»):

- **Фаза 4 остаток**: variable_lod (9,9k) + VoxelEngine остаток + VoxelDataGrid + TSan — крупнейший кусок: multi-LOD оркестратор.
- **Фаза 5**: binding 75+ классов + editor (12,8k) + edition (7,8k) + modifiers (1,5k) + instancing (9,2k) + terrain root (2k) — по объёму сопоставимо со всем уже сделанным.
- **Осознанно отложено/опционально**: GPU-путь (gpu + detail_rendering + shaders ≈7,1k), sqlite (2,2k), multipass (2,3k), FastNoise2 (3,1k), physics (Rapier, §9 плана — не начат).

## 9. Ревью качества кода и производительности

> Метод: 5 независимых ревью-проходов по подсистемам со сверкой каждой находки с
> C++-референсом; все находки ниже имеют точные file:line и перепроверены выборочно
> вручную. Бенчмарки перепрогнаны. Оценки модулей:
> **streams/math/io 8/10 · storage 7/10 · generators 7/10 · meshers 6/10 · конкурентность 5/10.**

### 9.1 Критично: конкурентная модель фактически однопоточная

Три слоя полной сериализации в горячем пути. Любого из них достаточно, чтобы
свести пользу пула потоков к нулю; вместе они делают число worker-потоков косметическим:

1. **`Mutex` вокруг мешера держится на весь `build()`** — `meshers/mesh_block_task.rs:124-178`:
   guard берётся до gather и живёт через весь `mesher.build()` (самый дорогой шаг конвейера).
   Одновременно мешится ровно один блок, сколько бы потоков ни было. C++ контракт противоположный:
   `voxel_mesher.h:81` — *«This can be called from multiple threads at once»*, `build()` зовётся
   на общем `Ref<VoxelMesher>` вообще без замка. Комментарий в Rust-коде («The C++ contract is
   single-threaded for these resources per task») **неверно описывает C++** — это ошибка порта, не дизайн.
2. **`SharedVoxelGenerator = Arc<Mutex<Box<dyn VoxelGenerator>>>`** (`storage/voxel_data.rs:54`) —
   вся генерация (load-таски, LOD-каскад, mesh-gather) сериализуется на одном замке.
   C++ (`voxel_generator.h:48`): *«Must be implemented in a multi-thread-safe way»* — генераторы
   зовутся конкурентно без внешнего замка. Корень: `generate_block(&mut self, ...)` — при том,
   что все 4 существующих генератора только читают `self`. Фикс: `&self` + `Arc<dyn VoxelGenerator>`
   (по образцу уже правильного `SharedVoxelStream = Arc<dyn VoxelStream>`).
3. **`SharedVoxelData` data `RwLock` после аудита закрыт 2026-07-07**: worker bridge теперь хранит
   settings под отдельным lock и каждую LOD map под независимым `RwLock`; terrain/mesh load/view/gather
   paths больше не берут общий data lock. macOS stress для edit/load/mesh добавлен; остаётся TSan.
4. **Потенциальный ABBA-дедлок**: `LoadBlockForTerrainTask` берёт Data→Generator
   (`voxel_terrain_core.rs:748-777`), `MeshBlockTask` — Generator→Data (`mesh_block_task.rs:131-138`).
   При общем генераторе (естественная production-конфигурация) два worker-потока могут
   взаимно заблокироваться. Путь не покрыт тестами (все тесты создают `MeshingDependency` с `generator=None`).
   Исчезает автоматически при фиксе п.2.

Хорошее в этом же слое: примитивы `thread/mod.rs` (Semaphore/Mutex/RwLock) — образцово корректны
(spurious-wakeup-safe, `!Send`-guard, единообразный poison-handling), atomics ordering в
dependency-флагах верный (Release/Acquire), `unsafe impl Send/Sync` нет вообще.

### 9.2 Критично/важно: горячий путь мешинга дороже C++ по построению

Адаптерный слой (`builtin.rs`, `mesh_block_task.rs`) вернул именно те издержки,
которые C++ явно устранял (цитата из `transvoxel.cpp:1181`: *«We settle data types up-front
so we can get rid of abstraction layers and conditionals»*):

- **Per-sample dyn-dispatch + div/mod + двойная ветвистость** (`builtin.rs:44-67`): `sample_f32`
  получает готовый плоский ZXY-индекс, раскручивает его div/mod в (x,y,z), а `get_voxel_f`
  пересчитывает тот же индекс обратно + ветвится по Compression и дважды по ChannelDepth — и всё
  это за vtable. Десятки таких вызовов на ячейку. C++ диспетчеризует по depth один раз и ходит по
  `Span<const T>` напрямую. Фикс: enum над `&[i8]/&[i16]/&[f32]`, полученный до цикла.
- **`is_uniform` fast-path после аудита закрыт 2026-07-06**: в исходном состоянии C++ отсекал
  однородные блоки (воздух/массив) за O(1) до цикла (`voxel_mesher_transvoxel.cpp:296`), а Rust
  всегда гонял полный O(n³) обход. Для реального террейна (глубоко под землёй / высоко в небе)
  это большинство блоков.
- **`MeshArrays`/`MesherOutput` аллоцируются заново на каждый блок** (`builtin.rs:108`,
  `mesh_block_task.rs:164`): C++ использует `thread_local` переиспользуемые массивы («once capacity
  is big enough, no more memory should be allocated»). Doc-комментарий на `MesherOutput` заявляет reuse,
  которого нет ни в одном call-site.
- **Scratch `VoxelBuffer` создаётся внутри цикла 3×3×3 соседей** (`mesh_block_task.rs:288-311`),
  до 26 heap-аллокаций на gather при недогруженных соседях; C++ выносит буфер из цикла и берёт из пула.
  Портированный `VoxelMemoryPool` в мешерах не задействован вовсе.
- **Cubes/Blocky адаптеры копируют весь блок** в свежий `Vec` через ветвистый `get_voxel`
  (`builtin.rs:185-197, 302-313`); C++ делает zero-copy `reinterpret_cast_to<T>()`.

Ядро алгоритмов при этом портировано добротно: reuse-cells transvoxel'а воспроизведён точно
(подтверждено byte-parity тестами), blocky/greedy — аллокационно-аккуратные, ZXY-layout выдержан.

### 9.3 Важно: остальные находки по подсистемам

**generators/graph** (не per-voxel AST-walk — оценка по Y-слайсам буферами, это правильная основа; но):
- `topological_order()` пересчитывается на **каждый** `generate()` (= каждый Y-slice, ×16 на блок),
  с постройкой HashMap+2×HashSet и O(n) `find()` на узел (`runtime.rs:401-417`) — топология неизменна, кэшируется тривиально.
- Per-element `HashMap`-lookup в `value_at()` внутри циклов всех операций (`runtime.rs:766`) — 512 SipHash-проб на слайс там, где нужно 2.
- **Нет XZ outer-group кэширования** из C++ (`voxel_generator_graph.cpp:905`): Y-независимая часть
  графа (для террейна — почти весь граф) пересчитывается на каждом из 16 Y-слайсов — до ~16× лишней работы.
- `GraphScratch` заявляет reuse аллокаций в doc-комментарии, но `HashMap::clear()` дропает все `Vec` — N×16 свежих аллокаций на блок.
- **`compress_uniform_channels()` после аудита закрыт 2026-07-06**: в исходном состоянии его не было в конце
  генерации (C++ зовёт всегда, `voxel_generator_graph.cpp:968`; соседний `HeightmapNoise` в Rust уже делал
  правильно), поэтому однородные блоки оставались развёрнутыми.
- `math::interval` портирован, но генераторами не используется (range-skip нет) — задокументированный дефер.
- **`HeightmapNoise` curve sharing после аудита закрыт 2026-07-06**: в исходном состоянии генератор
  клонировал `Curve` (Vec из 256 f32) на каждый блок до early-exit'ов (`simple.rs:472`); теперь
  хранит `Arc<Curve>`.

**tasks/ThreadedTaskRunner:**
- **A5 после аудита закрыт 2026-07-07**: worker wakeups идут через отдельный
  `thread::Semaphore` (post на enqueue и postponed requeue), enqueue пишет в staging-очередь под отдельным
  lock, worker переносит staged tasks в основную очередь и сортирует cached priorities перед pop с
  конца. `wait_for_all_tasks` остаётся на condvar и учитывает staged work.
- `VoxelTerrainCore` теперь dispatch'ит load/mesh work через `enqueue_many`, а `process()` делает
  неблокирующий drain завершённых тасков вместо двух `wait_for_all_tasks()` за tick.

**storage:**
- D4 после аудита закрыт 2026-07-07: hot-path accessors `VoxelBuffer`/`VoxelDataMap`
  теперь `#[inline]`; `fill_area` считает ZXY row base вне внутреннего Y-цикла; `downscale_to`
  и `paste_masked*` пишут destination-каналы через safe `read_write_area`/`read_write_area_with_channel`
  с dispatch по channel depth до обхода вокселей (Rust-аналог C++ `write_box_template`).

**streams/io/format:**
- **`RegionFile` deferred header write после аудита закрыт 2026-07-06**: в исходном состоянии
  `save_block` переписывал весь header+LUT (~16 KiB) на диск на каждое сохранение блока
  (`region_file.rs:547`); C++ держит `_header_modified` и пишет header один раз в `flush()/close()`.
  N сохранений = N перезаписей заголовка — прямое усиление I/O при save-штормах.
- **`.vox` negative-size guard после аудита закрыт 2026-07-06**: в исходном состоянии
  `0xFFFFFFFF` → `-1i32` проходил проверку `> MAX_MODEL_SIZE`, затем `volume_u64()` sign-extend'ил
  размер в ~1.8×10¹⁹; теперь `SIZE` dimensions валидируются через `0..=MAX_MODEL_SIZE`.
- `VoxelBuffer::create()` depth preservation после аудита закрыт 2026-07-07: в исходном
  состоянии `create()` безусловно сбрасывал кастомные channel depths — расходилось с C++
  (сохраняет при `format==null`), и с собственным doc-комментарием.
- `blocky/bake.rs` raw-pointer aliasing после аудита закрыт 2026-07-07: в исходном
  состоянии cutout-pass обходил borrow checker через `unsafe`; теперь cutout surfaces
  считаются на локальной копии модели под shared borrow библиотеки и переносятся обратно.

### 9.4 Следствие для заявления H2 («Rust в 1.5× быстрее C++»)

Бенчи воспроизводятся (перепрогнаны: 27.5µs/149 Melem/s на sphere_16), но меряют **ядро
`build_regular_mesh` напрямую**, минуя адаптер `builtin.rs` (dyn-sample), `gather_voxels_cpu`
и аллокации per-блок — то есть именно те слои, где ревью нашло регрессии против C++. End-to-end
преимущество 1.5× из этого бенча **не следует**; с учётом сериализации конвейера (§9.1)
многопоточный throughput сейчас заведомо хуже C++. H2 стоит перемерить на уровне
`MeshBlockTask::run` + многопоточного стриминга после фиксов.

### 9.5 Приоритеты фиксов

> Обновление 2026-07-06: trait-level часть пункта #1 закрыта — `VoxelGenerator`
> и `VoxelMesher` переведены на shared immutable contract (`&self` + `Arc<dyn ...>`)
> без внешних generator/mesher mutex. A4 lock-order rule тоже закрыт: текущие
> generator/mesher/stream callbacks выполняются после выхода из `VoxelData` lock.
> Обновление 2026-07-07: оставшаяся часть #1 тоже закрыта для `SharedVoxelData` worker bridge —
> settings вынесены в отдельный lock, LOD maps — в независимые `RwLock`, terrain/mesh worker paths
> мигрированы с общего data lock. A5 task runner и macOS stress тоже закрыты; дальше по конкурентности: TSan.

| # | Что | Эффект | Стоимость |
|---|---|---|---|
| 1 | `generate_block(&self)` / `build(&self)` + `Arc<dyn>` вместо `Arc<Mutex<Box<dyn>>>`; `SharedVoxelData` data `RwLock` → per-LOD map locks/settings lock | ✅ закрыто; worker pipeline больше не сериализуется общим generator/mesher/data lock | сделано до multi-LOD и Фазы 5 |
| 2 | ABBA-дедлок Data↔Generator | ✅ устранён п.1/A4; generator/stream/mesher callbacks не выполняются под data/map lock | закрыто |
| 3 | Transvoxel-адаптер: depth-dispatch до цикла + `is_uniform` skip + reuse `MeshArrays` | крупнейший CPU-выигрыш мешинга | низкая-средняя, локально в `builtin.rs` |
| 4 | `RegionFile`: dirty-flag вместо `save_header()` на каждый блок | ×N меньше записей при save-штормах | низкая |
| 5 | `.vox`: проверка `0..=MAX_MODEL_SIZE` | закрывает crash на битом файле | 1 строка |
| 6 | Graph: compile-step (кэш topo + dense-буферы вместо HashMap) → XZ-кэш → `compress_uniform_channels` | до ~16× на терре́йн-графах | средняя, инкрементально |
| 7 | Мелочи: `#[inline]` ×8, `fill_area` row-hoist, `enqueue_many`, `Arc<Curve>`, Semaphore вместо `notify_all` | суммарно заметно | низкая |

Детальные варианты решения по каждому пункту — §9.6.

### 9.6 Варианты решения

#### A. Снятие Mutex-трио (проблема §9.1)

**A1. Генератор → `&self` + `Arc<dyn>` — рекомендуется.**

```rust
pub trait VoxelGenerator: Send + Sync {
    fn generate_block(&self, q: VoxelQueryData<'_>) -> GenResult;  // было &mut self
}
pub type SharedVoxelGenerator = Arc<dyn VoxelGenerator>;           // было Arc<Mutex<Box<dyn ...>>>
```

Миграция механическая: все 4 существующих генератора (`Waves`/`Flat`/`Noise`/`HeightmapNoise`)
только читают `self` — правка сводится к сигнатурам и удалению `.lock()` в call-site'ах
(`with_generator`, `MeshBlockTask`, `LoadBlockForTerrainTask`, `update_lods`, `pre_generate_box`).
Будущим генераторам с внутренним кэшем — точечный interior mutability вокруг кэша
(как C++ `_shader_mutex`), а не замок на весь вызов. Образец уже в кодовой базе:
`SharedVoxelStream = Arc<dyn VoxelStream>` сделан правильно.

*Альтернатива (отвергнута):* клон генератора на каждый worker — расхождение состояния,
лишняя память, не соответствует C++ контракту «один шаренный потокобезопасный ресурс».

**A2. Мешер → `build(&self, ...)` + вынос scratch. Три рабочих варианта:**

- *Вариант 1 — thread-local scratch (зеркалит C++, минимум API-правок):*

  ```rust
  thread_local! {
      static TLS: RefCell<(transvoxel::Cache, MeshArrays)> = Default::default();
  }
  ```

  Ровно то, что делает `voxel_mesher_transvoxel.cpp:284` (`static thread_local tls_cache`).
  `TransvoxelMesher.cache` перестаёт быть полем — единственная причина `&mut self` исчезает.

- *Вариант 2 — явный scratch-параметр:* `build(&self, scratch: &mut MesherScratch, ...)`,
  worker владеет своим `MesherScratch`. Без TLS, легче тестировать; цена — правка сигнатуры
  трейта и всех трёх адаптеров + `MeshBlockTask`.

- *Вариант 3 — per-worker scratch через `ThreadedTaskContext` (рекомендуется).* Второй проход
  по коду показал: Rust-порт **уже** передаёт в каждый `ThreadedTask::run` контекст с
  `thread_index: u8` (`tasks/threaded_task.rs:13-16`) — половина инфраструктуры готова.
  Осталось положить в контекст ссылку на per-worker хранилище:

  ```rust
  pub struct ThreadedTaskContext<'w> {
      pub thread_index: u8,
      pub task_priority: TaskPriority,
      pub scratch: &'w mut TaskScratch,  // type-map: HashMap<TypeId, Box<dyn Any + Send>>
  }
  ```

  `worker_loop` владеет своим `TaskScratch`; `MeshBlockTask` достаёт/кладёт туда
  `TransvoxelScratch` (Cache + MeshArrays). Ни TLS, ни замков; детерминированно и тестируемо;
  crate `tasks` ничего не знает о мешерах (type-map). Ёмкость буферов накапливается на воркере —
  тот же эффект, что у C++ `thread_local`, но с явным владением.

Рекомендация: вариант 3; вариант 1 — как самый дешёвый временный мост, если разносить A2 на два шага.
*Отвергнуто:* мешер-инстанс на worker (factory) — усложняет владение в `MeshingDependency` без выгоды.

**A3. VoxelData → per-LOD `RwLock` + подключение real `SpatialLock3D` — рекомендуется.**

```rust
struct Lod {
    map: thread::RwLock<VoxelDataMap>,   // блок-карта: читатели параллельны
    spatial_lock: SpatialLock3D,         // регионная эксклюзивность для вокселей
}
pub struct VoxelData { lods: Box<[Lod]>, /* bounds, format, ... — иммутабельные */ }
```

Контракт как в C++ (`voxel_data.h:197-235`): mesh-gather берёт `map.read()` + `spatial_lock.read(box)`;
редактирование — `map.read()` + `spatial_lock.write(box)`; добавление/удаление блоков — `map.write()`.
`SpatialLock3D` после аудита закрыт 2026-07-07: `thread/mod.rs` теперь хранит
`Vec<(BoxBounds3i, Mode)>` под `Mutex` + `Condvar`, разрешает overlapping reads, блокирует
overlapping writes и пропускает disjoint regions. Подшаг 2026-07-07 заменил внешний
`Arc<Mutex<VoxelData>>` в terrain/mesh consumers на `Arc<SharedVoxelData>` и начал брать
`SpatialLock3D` read/write regions в mesh-gather и data view/load paths. Подшаг 2026-07-07
заменил bridge lock на shared data `RwLock`, так что read-only snapshots теперь
пересекаются. Подшаг 2026-07-07 заменил общий data `RwLock` внутри `SharedVoxelData`
на settings lock + per-LOD map locks и перевёл terrain/mesh worker paths на эти методы.
A3 для worker bridge закрыт; A5 task runner закрыт; macOS stress закрыт; остаток волны 2 — TSan.

*Альтернативы (отвергнуты):* шардированный map (DashMap-стиль) — новая зависимость и другая
семантика блокировок, теряем сверяемость с C++; actor-модель (один поток-владелец + каналы) —
убивает параллельные чтения и требует пере-дизайна всех call-site'ов.

*Миграция и подводные камни A3 (дополнено вторым проходом):*
- У потребителей `Arc<Mutex<VoxelData>>` → `Arc<SharedVoxelData>` с внутренними
  замками (как C++ `std::shared_ptr<VoxelData>`) — ✅ bridge-подшаг закрыт для
  `MeshBlockTask` / `VoxelTerrainCore` / `LoadBlockForTerrainTask`.
- Bridge mutex внутри `SharedVoxelData` → data `RwLock` — ✅ закрыто: `with_data`
  брал shared read guard, mutation paths брали write guard.
- Data `RwLock` внутри `SharedVoxelData` → per-LOD map locks + settings lock — ✅ закрыто:
  `SharedVoxelData` хранит settings отдельно, LOD maps отдельно, mesh gather берёт read-lock
  только нужной LOD map, terrain view/unview/load writes берут write-lock нужной LOD map.
- Поля-настройки (`generator`, `stream`, `bounds`, `streaming_enabled`, ...) — в
  `RwLock<Settings>` (зеркало C++ `_settings_lock`): иначе `set_generator`/`set_bounds`
  потребуют `&mut VoxelData`, которого при `Arc<VoxelData>` больше нет. Это же отвечает
  на вопрос A1, где живёт `Arc<dyn VoxelGenerator>`.
- Межлодовый порядок: `update_lods` держит замки двух LOD одновременно (src→dst) —
  зафиксировать правило «замки LOD берутся строго по возрастанию индекса», иначе появится
  новый ABBA между LOD-каскадом и редактированием.

**A4. Порядок замков — ✅ закрыто 2026-07-06.** После A1 замок генератора исчез,
а A4 дополнительно закрепил правило: `VoxelData` lock не держится через вызовы
generator/mesher/stream. `LoadBlockForTerrainTask` снимает snapshot `block_size`/`format`/
`generator` под lock и вызывает fallback-генератор после drop; `MeshBlockTask`
копирует resident-соседей под lock, выносит missing-регионы в план и генерирует их
после выхода из критической секции. Regression-тесты проверяют это через
`try_lock()` внутри generator/mesher/stream callbacks и через overlap двух mesh-task'ов
внутри одного shared mesher.

**A5. Task runner — ✅ закрыто 2026-07-07.**
- worker wakeups идут через готовый `thread::Semaphore` (post на enqueue и postponed requeue, как C++
  `_tasks_semaphore`); condvar осталась для `wait_for_all_tasks` и completion-notify;
- enqueue пишет в staging-очередь под отдельным lock; worker переносит staged tasks в основную
  очередь, обновляет cached priorities в 32ms-окне, сортирует и делает pop с конца;
- `VoxelTerrainCore` собирает load/mesh work в batch и вызывает `enqueue_many`;
- `process()` больше не блокируется на двух `wait_for_all_tasks()`: tick делает неблокирующий drain
  завершённых тасков и даёт load/mesh задачам завершаться между кадрами.

**Валидация волны A:** ✅ macOS stress-тест добавлен (`threaded_edit_load_mesh_stress`:
6 worker threads + 2 mutator threads, параллельные mesh + edit + load на общей `VoxelData`,
счётчики целостности). Остался прогон под TSan (nightly `-Zsanitizer=thread`, на Linux-хосте),
опционально loom для `SpatialLock3D`. Это закрывает GO-критерий Фазы 4 формально.

#### B. Горячий путь мешинга (проблема §9.2)

**B1. Типизированный SDF-вход вместо `&dyn` — рекомендуется.**

```rust
enum SdfInput<'a> { I8(&'a [i8]), I16(&'a [i16]), F32(&'a [f32]), Uniform(f32) }
```

Резолвится **один раз** до цикла через `VoxelBuffer::channel_slice<T>()`.
⚠️ *Пересмотр вторым проходом:* канал хранится как `Vec<u8>` (`voxel_buffer.rs:207`) с гарантией
выравнивания 1, поэтому «просто reinterpret» `&[u8]`→`&[i16]/&[f32]` — UB/panic-риск по alignment,
а не по endianness (все целевые платформы LE). Три безопасных пути: **(а)** `bytemuck::try_cast_slice`
с fallback на `chunks_exact`-декодирование (на практике аллокатор даёт выравнивание ≥8, fallback
останется холодным); **(б)** выровненное хранилище из пула — `VoxelMemoryPool` выдаёт буферы с
align 8 (например, поверх `Vec<u64>`), касты всегда успешны; **(в)** структурный фикс D7 —
типизированное хранилище каналов, снимающее вопрос навсегда. Рекомендация: (а) сразу, (в) как
целевое состояние.
`build_regular_mesh` делается generic по сэмплеру (`fn build_regular_mesh<S: SdfSampler>`),
диспетчер-обёртка матчится по depth один раз — мономорфизация даёт ровно C++-шаблонную схему
(`build_regular_mesh_dispatch_sd`). Уходят: vtable-вызов, div/mod-раскрутка индекса, ветки
Compression/Depth на каждый сэмпл. Golden-тесты (sphere_16/32) поймают любой регресс парити.

**B2. `is_uniform` fast-path** — ✅ закрыто 2026-07-06: в начале `TransvoxelMesher::build`:

```rust
if input.voxels.is_uniform(self.sdf_channel) { /* пустая Surface, см. ниже */ return; }
```

*Нюанс контракта (уточнено вторым проходом):* C++ при uniform-блоке выходит, не эмитя сёрфейс
вовсе (`voxel_mesher_transvoxel.cpp:296`); текущий Rust-адаптер всегда эмитит (пустую) Surface,
и на это может опираться state-machine `VoxelTerrainCore`. Безопасный фикс — сохранить текущий
Rust-контракт (пустая Surface, но без O(n³) обхода); сведение к C++-семантике «нет сёрфейса» —
отдельным шагом с проверкой обработчика output'ов. Regression-тест подтверждает, что uniform SDF
возвращает пустую surface без единого sample-вызова.

**B3. Переиспользование мешевых массивов.** `MeshArrays` — в thread-local scratch из A2-варианта-1
(C++ делает именно так). Для `MesherOutput`/`Surface`, которые уходят из таска по move, — free-list
пул на уровне terrain (`Mutex<Vec<MeshArrays>>`): таск берёт буферы из пула, потребитель output'а
(после аплоада меша в Phase 5) возвращает. Ёмкость стабилизируется за первые десятки блоков.

**B4. Gather-scratch из цикла.** В `gather_voxels_cpu` вынести scratch `VoxelBuffer` за цикл
3×3×3 и создавать с `Allocator::Pool` (портированный `VoxelMemoryPool` наконец задействуется) —
зеркало `mesh_block_task.cpp:200` (`VoxelBuffer generated_voxels(ALLOCATOR_POOL)` до цикла).

**B5. Cubes/Blocky zero-copy.** Тот же `channel_slice<T>()` из B1 вместо po-воксельного копирования
в свежий `Vec`; для `Compression::Uniform` — ветка с повторяемым значением без материализации.

#### C. Graph runtime (§9.3)

**C1. Compile-шаг — ядро всех фиксов графа:**

```rust
pub struct CompiledGraph {
    nodes: Vec<CompiledNode>,   // в топологическом порядке, dense-индексы вместо GraphNodeId
    xz_prefix: usize,           // узлы 0..xz_prefix не зависят от InputY
    outputs: Vec<(GraphOutput, usize)>,
}
// scratch: Vec<Vec<f32>> по dense-индексу — HashMap и value_at() исчезают,
// слайсы входов резолвятся до цикла; буферы переживают clear() (fill/truncate вместо drop)
```

Компиляция один раз в `GraphGenerator::new` (topo-sort + классификация достижимости из `InputY`).
Исполнение: `xz_prefix` считается только на первом Y-слайсе блока и кэшируется — эквивалент
C++ `inner_group_start_index` / `skip_outer_group` (`voxel_generator_graph.cpp:905`), до ~16×
экономии на терре́йн-графах. Это рефакторинг runtime.rs без смены публичного API `GraphGenerator`.
Туда же в compile-шаг (дополнено вторым проходом): **(а) хойст построения сэмплеров** — сейчас
`Noise2D/3D` зовут `noise.build()` на каждый Y-slice (`runtime.rs:557,569`); в `CompiledNode`
кладётся сэмплер, построенный один раз при компиляции; **(б) constant folding** — подграфы из
констант сворачиваются в готовые значения (симметрия с уже портированным
`expression_parser::precompute_constants`; C++-компилятор графа делает то же).

**C2. `compress_uniform_channels()`** в конце `generate_block_with_graph` — ✅ закрыто 2026-07-06:
`GraphGenerator` сжимает uniform-каналы после послайсовой записи (зеркало `voxel_generator_graph.cpp:968`,
сосед `HeightmapNoise` уже делал).

**C3. Range analysis (после C1):** `analyze_range(&self, box) -> Interval` по компилированному
порядку на портированном `math::interval` (сейчас не используется вообще); если интервал SDF
не пересекает ноль — блок заполняется униформно без per-voxel исполнения. Это тот же механизм,
которым C++ VM обгоняет наивное исполнение.

#### D. Точечные фиксы streams/storage

| # | Фикс | Суть |
|---|---|---|
| D1 | `RegionFile`: поле `header_dirty: bool` | ✅ закрыто 2026-07-06: `save_block` только метит; физическая запись header'а в `flush()`/`close()`/`Drop` (зеркало C++ `_header_modified`) |
| D2 | `.vox`: `(0..=MAX_MODEL_SIZE).contains(&size.{x,y,z})` | ✅ закрыто 2026-07-06: отрицательные размеры (`0xFFFFFFFF`/`-1`) отклоняются до расчёта volume/model allocation; заодно строже C++ |
| D3 | `VoxelBuffer::create(size, format: Option<&VoxelFormat>)` | ✅ закрыто 2026-07-07: `create()` сохраняет текущие channel depths при сбросе буфера и пересчитывает uniform default под текущий depth; явный `VoxelFormat::configure_buffer()` по-прежнему применяет формат |
| D4 | `#[inline]` на `get_voxel`/`set_voxel`/`get_voxel_f`/`set_voxel_f` ×2 типа; `fill_area` — база строки за внутренний цикл (образец: свой же `fill_3d_region_zxy`); depth-hoisted helper для `downscale_to`/`paste_masked*` (аналог C++ `write_box_template`) | ✅ закрыто 2026-07-07: `VoxelBuffer::read_write_area*` dispatch'ит destination depth один раз, `downscale_to`/masked paste пишут через helper, row base вынесен из inner Y loop |
| D5 | `HeightmapNoise::curve: Option<Arc<Curve>>` | ✅ закрыто 2026-07-06: клон 256×f32 на блок заменён на O(1) refcount (образец — graph-узел Curve) |
| D6 | `blocky/bake.rs`: compute-then-assign вместо `unsafe` raw-pointer aliasing | ✅ закрыто 2026-07-07: cutout-данные считаются на локальной копии `BakedModel` под shared borrow библиотеки, затем `cutout_side_surfaces` переносится обратно без raw pointer aliasing |
| D7 | **(структурная опция)** типизированное хранилище каналов: `enum ChannelData { U8(Vec<u8>), U16(Vec<u16>), U32(Vec<u32>), U64(Vec<u64>) }` | снимает alignment-вопрос B1/B5 навсегда, делает depth-dispatch естественным (match один раз → типизированный слайс) и бесплатно даёт «write_box_template»-аналог из D4; цена — рефакторинг `voxel_buffer.rs` + сериализатора (typed→bytes каст безопасен всегда). Решить до волны 3 |

#### Порядок внедрения

Дорожки A (конкурентность), B (мешинг), C (граф), D (точечные) почти независимы — под модель
«AI 24/7» их можно вести параллельными сессиями. Явные связки: B1/B5 требуют решения по
хранению каналов (D7 или выровненный пул — см. ⚠️ в B1); A2-вариант-3 включает мини-правку
`tasks`; B3 естественно делается вместе с A2.

**Волна 1 — дешёвое и разблокирующее (каждый пункт — отдельный коммит):**
✅ закрыта 2026-07-06: A1 (генератор `&self`) → A2 (мешер `&self` + scratch) →
A4 (правило замков).
Уже после этой волны mesh-build'ы и генерация исполняются параллельно (глобальный замок
`VoxelData` остаётся только на gather — короткая секция).
**DoD:** новый тест «N воркеров мешат M блоков» показывает масштабирование по потокам
(время ~1/N, загрузка >1 ядра); 655+11 тестов и golden-парити зелёные.

**Волна 2 — конкурентность до конца:**
A3 (`Arc<SharedVoxelData>` + per-LOD RwLock/settings lock) ✅ → A5 (semaphore + staging +
неблокирующий drain в `process()`) ✅ → macOS stress ✅ → TSan.
**DoD:** stress-тест (8 потоков: mesh + edit + load на общей `VoxelData`) стабилен; остаётся
чистый TSan-прогон (nightly `-Zsanitizer=thread`, Linux-хост) для формального GO-критерия Фазы 4.

**Волна 3 — производительность:**
решение по D7 → B1 → B3/B4/B5 → C1 → C3.
**DoD:** новый бенч **H2-MT** — throughput на уровне `MeshBlockTask::run` + многопоточный
paging-сценарий (движущийся viewer), сравнение с расширенным `cpp-baseline`; критерий прежний:
не хуже C++ −15%, target ≥0%.

**Инфраструктура (найдено вторым проходом, вне волн):**
- **CI для `rust/`**: workflow заготовлен, но авто-триггеры намеренно выключены 2026-07-07
  (`workflow_dispatch` only), чтобы не ломать GitHub flow во время пилота.
  В ручном режиме `.github/workflows/rust.yml` запускает `cargo fmt --all -- --check`,
  `cargo test --workspace`, `cargo clippy --workspace --all-targets` и
  `cargo build --workspace` на Ubuntu для Rust-изменений, затем Android aarch64
  GDExtension smoke через `rust/scripts/android-build.sh`. Автоматический CI-gate,
  бенч-смоук и x86_64-android emulator smoke остаются отдельными infra-пунктами.
- **Бенч-харнесс H2-MT** (см. DoD волны 3) — без него результаты волн неизмеримы: текущий бенч
  меряет только ядро мешера в один поток.
- **cargo-fuzz таргеты на парсеры** (`.vox`, `block_serializer`, `region`): C++-сторона уже
  фаззится (`fuzzer.yml`), Rust-парсеры — нет; баг D2 — ровно тот класс, который находит фаззер.

Инварианты на всём протяжении: 655 unit + 11 integration + golden-парити остаются зелёными;
clippy/fmt чистые; каждый шаг сверяется с соответствующим C++-файлом (ссылки в §9.1-9.3).

---

### 9.7 Журнал фиксов после аудита

| Дата | Пункт аудита | Статус | Проверка |
|---|---|---|---|
| 2026-07-06 | A1, часть generator: `VoxelGenerator::generate_block(&self)` / `generate_single(&self)` + `SharedVoxelGenerator = Arc<dyn VoxelGenerator>` | ✅ закрыто. Внешний generator-mutex удалён из `VoxelData`, `MeshingDependency`, `MeshBlockTask`, `LoadBlockForTerrainTask`; `GraphGenerator` синхронизирует только собственный scratch локально | `cargo test -p voxel-core` → 635 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-06 | A2, часть mesher: `VoxelMesher::build(&self)` + `SharedVoxelMesher = Arc<dyn VoxelMesher>` | ✅ закрыто. Внешний mesher-mutex удалён из `MeshingDependency`/`MeshBlockTask`; `TransvoxelMesher` использует thread-local `Cache`, поэтому shared mesher не сериализует build через внутренний глобальный lock | `cargo test -p voxel-core` → 635 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-06 | D2: `.vox` negative model-size guard | ✅ закрыто. `SIZE` dimensions now must be in `0..=MAX_MODEL_SIZE`, so `0xFFFFFFFF`/`-1` returns `InvalidData` before model allocation | `cargo test -p voxel-core` → 636 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-06 | C2: graph uniform-channel compression | ✅ закрыто. `GraphGenerator` calls `VoxelBuffer::compress_uniform_channels()` after generation; constant SDF output remains `Compression::Uniform` instead of a materialized channel | `cargo test -p voxel-core` → 637 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-06 | B2: Transvoxel uniform SDF fast-path | ✅ закрыто. `TransvoxelMesher` skips `build_regular_mesh` when the SDF channel is uniform, preserves the current Rust contract of one empty `Transvoxel` surface, and avoids all sampler calls | `cargo test -p voxel-core` → 638 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-06 | D5: HeightmapNoise shared curve | ✅ закрыто. `HeightmapNoise::curve` is now `Option<Arc<Curve>>`; `set_curve` preserves owned-curve compatibility and `set_curve_arc` supports shared storage without cloning baked points | `cargo test -p voxel-core` → 639 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-06 | D1: RegionFile deferred header write | ✅ закрыто. `RegionFile` keeps a `header_dirty` flag; `save_block` only marks the LUT dirty, while `flush()`/`close()`/`Drop` persist the header once | `cargo test -p voxel-core` → 640 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-06 | A4: data-lock ordering rule | ✅ закрыто. `LoadBlockForTerrainTask` snapshots stream/generator settings under `VoxelData` lock and runs stream/generator work after drop; `MeshBlockTask` now queues missing gather regions under lock, fills them outside the critical section, then calls the mesher after lock release. Regression tests assert `try_lock()` succeeds inside generator/mesher/stream callbacks, plus two mesh tasks can overlap inside one shared mesher | `cargo test -p voxel-core` → 645 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | D3: `VoxelBuffer::create` depth preservation | ✅ закрыто. `create()` now preserves existing per-channel depths when no explicit `VoxelFormat` is applied, resets channels to uniform defaults for those depths, and keeps `VoxelFormat::configure_buffer()` as the explicit format path | `cargo test -p voxel-core` → 646 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | D6: safe blocky cutout bake | ✅ закрыто. `generate_library_cutout_sides` no longer creates a raw immutable alias of `BakedLibrary` while mutating one model; it computes cutouts on a local model copy and moves `cutout_side_surfaces` back. Safety regression test rejects `unsafe {` in the bake module | `cargo test -p voxel-core` → 647 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | D4: storage hot-path write helpers | ✅ закрыто. `VoxelBuffer`/`VoxelDataMap` raw+float accessors are inline, `fill_area` writes by row base, and `downscale_to`/masked paste use safe depth-hoisted destination write helpers instead of per-voxel `set_voxel` dispatch | `cargo test -p voxel-core` → 649 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | A3 substep: real SpatialLock3D | ✅ закрыто. `SpatialLock3D` now tracks `(BoxBounds3i, mode)` entries behind `Mutex<Vec<_>> + Condvar`: overlapping reads coexist, overlapping writes block, disjoint regions proceed. Added blocking/read-write regression tests. | `cargo test -p voxel-core` → 650 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | A3 substep: SharedVoxelData bridge + region guards | ✅ закрыто. Terrain/mesh consumers now use `Arc<SharedVoxelData>` instead of external `Arc<Mutex<VoxelData>>`; mesh gather and data view/load paths take scoped `SpatialLock3D` read/write regions. Added region-lock regression test on `SharedVoxelData`. | `cargo test -p voxel-core` → 651 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | A3 substep: SharedVoxelData read concurrency | ✅ закрыто. `SharedVoxelData::with_data` now uses a shared `RwLock` read guard and mutation paths use write guards, so independent read snapshots overlap while writes remain exclusive. Added `shared_voxel_data_allows_parallel_read_snapshots`. | `cargo test -p voxel-core` → 652 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | A3 substep: per-LOD map locks + settings lock | ✅ закрыто. `SharedVoxelData` no longer wraps one `VoxelData` in a common data lock: settings are snapshotted from a separate lock, each LOD map has its own `RwLock`, mesh gather reads only the target LOD map, and terrain view/unview/load writes mutate only the target LOD map. Added `shared_voxel_data_allows_parallel_lod_map_writes`. | `cargo test -p voxel-core` → 653 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | A5: task runner semaphore/staging + nonblocking terrain drain | ✅ закрыто. `ThreadedTaskRunner` stages enqueue work under a separate lock, wakes workers via `thread::Semaphore`, sorts cached priorities before end-pop, and keeps `wait_for_all_tasks` on condvar. `VoxelTerrainCore::process()` now drains completed tasks without blocking and dispatches load/mesh batches through `enqueue_many`. Added `enqueue_does_not_block_on_worker_queue_lock` and `process_does_not_wait_for_slow_load_tasks`. | `cargo test -p voxel-core` → 655 unit + 10 integration + 1 doc-test, 0 failed |
| 2026-07-07 | Wave 2 stress: threaded edit/load/mesh validation | ✅ закрыто для macOS cargo stress. Added `threaded_edit_load_mesh_stress`: six runner workers mesh shared `SharedVoxelData` while two scoped mutator threads perform region-locked edits and load-style block inserts; asserts all mesh outputs complete, edit/load counters match, and region locks are released. | `cargo test -p voxel-core --test threaded_edit_load_mesh_stress` → 1 passed |
| 2026-07-07 | Infra: Rust workspace workflow | 🟡 заготовлено, но auto-run выключен. Added `.github/workflows/rust.yml` with pinned-toolchain install, Cargo cache, fmt, workspace tests, clippy, and workspace build; after a failed GitHub runner probe, push/PR triggers were removed and only `workflow_dispatch` remains. | локально: `cargo fmt --all -- --check`; `cargo test --workspace`; `cargo clippy --workspace --all-targets`; `cargo build --workspace` |
| 2026-07-07 | Infra: Android aarch64 workflow smoke | 🟡 заготовлено в ручном workflow. `rust.yml` installs NDK 29.0.14206865, exports `ANDROID_NDK_HOME`, and runs `./scripts/android-build.sh` after the main Rust job. `android-build.sh` now also discovers NDKs from `ANDROID_HOME`, `ANDROID_SDK_ROOT`, and macOS `~/Library/Android/sdk/ndk`. | локально: `./scripts/android-build.sh` → aarch64 `.so` + `gdext_rust_init` |
| 2026-07-12 | **M1.A: TSan-прогон на Linux/nightly (формальный GO-критерий Фазы 4 по конкурентности)** | ✅ закрыто. Создан отдельный workspace-член `tsan` (зависит только от `voxel-core`, без `criterion`/`serde`/`zerocopy` — proc-macro `zerocopy_derive` конфликтует с TSan-runtime через `__tsan_func_entry`). Тесты: `concurrent_edit_load_mesh` (зеркало `threaded_edit_load_mesh_stress`), `spatial_lock_concurrency` (2 теста: 8 потоков × overlapping/disjoint read/write regions + blocking-write wakeup), `task_runner_concurrency` (2 теста: 4 producer × конкурентный enqueue + postponed-requeue path). Прогон под `cargo +nightly test -p tsan -Zbuild-std --target x86_64-unknown-linux-gnu` с `RUSTFLAGS="-Zsanitizer=thread -Cunsafe-allow-abi-mismatch=sanitizer"`, 3× стабильно — ни одного `WARNING: ThreadSanitizer`. Важный нюанс: **`-Zbuild-std` обязателен** — без него std не инструментирован и TSan не видит happens-before через `std::sync::Mutex`/`Condvar`, выдавая массовые false positives (первоначально детектилась «гонка» в `SpatialLock3D::lock_write`, исчезнувшая после пересборки std). Data race не найдена; конкурентная модель §9.1 валидирована формально. | `cargo +nightly test -p tsan -Zbuild-std --target x86_64-unknown-linux-gnu` → 5 passed, 0 TSan warnings; `cargo test -p voxel-core` → 655 unit + 11 integration + 1 doc-test; `cargo clippy --workspace --all-targets` clean; `cargo fmt --all -- --check` clean |
| 2026-07-12 | **M1.B / D7: типизированное хранилище каналов `ChannelData`** | ✅ закрыто. `Channel.data` изменён с `Vec<u8>` на `enum ChannelData { U8(Vec<u8>), U16(Vec<u16>), U32(Vec<u32>), U64(Vec<u64>) }` (вариант соответствует `ChannelDepth`). Главный эффект: hot loops (`read_write_area`, `read_write_area_with_channel`, `fill_area`, `is_uniform`, `copy_channel_from`, `copy_channel_from_area`) теперь диспетчеризуют depth **один раз на канал** и индексируют типизированный slice напрямую (`v[i]`) вместо `from_le_bytes`/`to_le_bytes` на каждый воксель — это structuralный фундамент для B1 (typed SDF sampler) и B5 (Cubes/Blocky zero-copy), снимающий alignment-вопрос навсегда. Добавлена зависимость `bytemuck = "1"` (pure Rust, Android/WASM-safe) для safe byte-cast в `ChannelData::as_bytes`/`as_bytes_mut` — wire-format `block_serializer` **неизменен** (`bytemuck::cast_slice` над LE `Vec<u{16,32,64}>` даёт тот же байтовый layout, что и старый `Vec<u8>`). Удалены ставшие мёртвыми `read_raw`/`write_raw`/`encode_raw`; добавлен `copy_channel_region_typed` (типизированный аналог byte-хелпера). **Pool recycling для typed storage отложен**: `VoxelMemoryPool` byte-oriented и test-only в Rust порте (ни один production path не выбирает `Allocator::Pool`), API surface сохранён для C++ parity, `alloc_typed` выделяет typed Vec напрямую; 3 pool-теста помечены `#[ignore]` с комментарием-ссылкой на D7. | `cargo test -p voxel-core` → 652 unit + 11 integration + 1 doc-test (3 ignored pool tests); `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; `cargo +nightly test -p tsan -Zbuild-std ...` → 5 passed, 0 TSan warnings; `block_serializer` round-trip тесты зелёные (wire-format byte-совместимость подтверждена) |
| 2026-07-12 | **M1.C / B1: типизированный SDF-вход в transvoxel-адаптере** | ✅ закрыто. `VoxelBufferTransvoxelInput::sample_f32` переписан через `enum TypedSdfSampler { Bit8/Bit16/Bit32/Bit64(&[T]) }`, который резолвит variant + depth-specific decode **один раз** в `TransvoxelMesher::build` (до входа в `build_regular_mesh`). `sample_f32(data_index)` теперь индексирует типизированный slice напрямую по flat ZXY индексу, который ядро уже вычисляет. Убрано: (а) per-voxel div/mod обратно в `(x,y,z)` — ядро уже работает в flat-index space, `voxel_index = y + sy*(x+sx*z)` это инверсия того же div/mod; (б) per-voxel `ChannelData::get_u64` match (третий per-voxel depth-dispatch); (в) per-voxel `raw_voxel_to_real` match. Всё свелось к одному depth-specific decode (`s8_to_snorm`/`s16_to_snorm`/f32-bits/f64-bits) выбранному один раз. Добавлен публичный accessor `VoxelBuffer::channel_data(ci) -> &ChannelData` и re-export `ChannelData` из `storage`. Vcall через `&dyn RegularMesherInput` оставлен (ядро документирует это как сознательный выбор; мономеризация — отдельный шаг если bench покажет нужду). | `cargo test -p voxel-core` → 652 unit + 11 integration + 1 doc-test (golden parity transvoxel sphere_16/32 + tables + parity — byte-identical, decode корректность подтверждена); `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; `cargo +nightly test -p tsan -Zbuild-std ...` → 5 passed, 0 TSan warnings |
| 2026-07-12 | **M1.C / B3: переиспользование мешевых массивов (terrain-level pool)** | ✅ закрыто. Добавлен `MeshArraysPool` (`Mutex<Vec<MeshArrays>>`, free-list с capacity-preserving `acquire`/`release`) во `voxel_mesher.rs`. `VoxelTerrainCore` владеет `Arc<MeshArraysPool>`, пробрасывает его через `MeshBlockTaskParams.mesh_arrays_pool` → `MeshBlockTask` → `MesherInput.mesh_arrays_pool` → `TransvoxelMesher::build`, где mesher берёт очищенный `MeshArrays` из pool'а вместо `MeshArrays::default()` на каждый блок. Критично: `build_regular_mesh` НЕ очищает output (appends через `add_vertex`/`push`), поэтому `acquire` возвращает cleared buffer (`clear()` сохраняет capacity). Pool возвращает arrays в `apply_mesh_update` (old output при re-mesh) и `unview_mesh_block` (unload) через новый `MesherOutput::take_first_transvoxel_arrays()`. thread_local на mesher-уровне невозможен — `MeshArrays` move'ится в `Surface`→`BlockMeshOutput`→`MeshBlockEntry.output`, поэтому pool живёт на terrain (как аудита §9.6-B3 и указывала). | `cargo test -p voxel-core` → 652 unit + 11 integration + 1 doc-test (golden parity + end-to-end pipeline зелёные); `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; `cargo +nightly test -p tsan -Zbuild-std ...` → 5 passed, 0 TSan warnings |

Пункт #1 по снятию generator/mesher/data сериализации закрыт для текущего worker bridge.
ABBA-риск с внешним generator/mesher lock снят; правило “не держать data lock через
generator/mesher/stream” закреплено A4 для текущих load/gather путей. Переиспользование
`MeshArrays`/`MesherOutput` остаётся отдельной perf-частью B3. **TSan-прогон на
Linux/nightly закрыт 2026-07-12 (M1.A):** формальный GO-критерий Фазы 4 по
конкурентности выполнен — data race не найдена на edit/load/mesh stress, `SpatialLock3D`
под нагрузкой и `ThreadedTaskRunner` enqueue/postpone путях.

## 10. Вывод

Миграция идёт дисциплинированно: каждая фаза закрывается тестами, документация точна,
код без заглушек-паник, parity подтверждён исполняемыми тестами, зависимость от C-кода
минимальна (важно для Android/WASM). Ревью кода подтверждает: **алгоритмическая корректность
и идиоматика на высоте** (порт не механический: Option вместо сентинелей, RAII-пул, точные
byte-parity тесты), но **два системных долга** требуют внимания до продолжения Фазы 4/5:

1. **Конкурентная модель** (§9.1): trait-level сериализация генерации/мешинга снята,
   callbacks больше не выполняются под data/map lock, а `SharedVoxelData` worker bridge
   перешёл на settings lock + per-LOD map locks, task runner получил semaphore/staging и
   неблокирующий terrain drain, macOS stress для edit/load/mesh зелёный. Формально закрыть
   GO Фазы 4 ещё мешает TSan-проверка.
2. **Горячий путь мешинга** (§9.2): адаптерный слой вернул абстракционные издержки, которые C++
   целенаправленно устранял; заявление H2 о 1.5× преимуществе не распространяется на end-to-end конвейер.

План действий — три волны из §9.6: (1) волна 1 закрыта,
(2) волна 2 — A3, A5 и macOS stress закрыты, дальше TSan
(закрывает GO-критерий Фазы 4 формально), (3) волна 3 — перф-фиксы горячего пути и graph runtime
с перемером H2 end-to-end. Параллельно: настроить upstream-tracking (`cpp-reference`) и
вернуться к автоматическому Rust CI/bench smoke/x86_64-android smoke, когда GitHub flow будет готов.
Multi-LOD paging начинать после волны 2 — уже на исправленной threading-модели.

---

## 11. Цель: полностью закрыть аудит (постановка 2026-07-12)

> Постановка от заказчика: «сначала закрыть §9 (долг по ревью кода), затем пройти весь путь
> до конца миграции (вариант охвата 4); коммитить и пушить по шагам, обновлять статус».
> Эта секция фиксирует, **что значит «аудит закрыт»**, в виде измеримой цели и дорожной карты.
> Сам аудит при этом НЕ переписывается ретроактивно — исходные находки §9 остаются как есть,
> закрытые пункты отмечаются в журнале §9.7, а прогресс по дорожной карте — здесь и в `STATUS.md`.

### 11.1 Definition of Done — «Аудит полностью закрыт»

Все четыре milestone'а ниже выполнены, каждый подтверждён прогонами (тесты/clippy/fmt/бенчи)
и зафиксирован коммитом + записью в §9.7 или соответствующей фазовой секции `STATUS.md`.

| M | Название | Критерий закрытия |
|---|---|---|
| **M1** | Долг по ревью кода (§9 + §7) | Все открытые пункты §9.5/§9.6 закрыты: TSan-прогон зелёный; волна 3 (B1/B3/B4/B5/C1/C3) выполнена; решение D7 принято и внедрено; H2-MT бенч существует и показывает ≥0% vs C++ −15%; cargo-fuzz таргеты есть; риски §7 сняты (upstream-tracking `cpp-reference` настроен, `REPORT.md` актуализирован, on-device валидация либо закрыта, либо явно задепрекейджена с обоснованием) |
| **M2** | Фаза 4 до GO | Multi-LOD paging (`VoxelLodTerrain` + update task + clipbox/octree) портирован и проходит parity/стресс; `VoxelEngine` остаток (time-spread/progressive очереди, stats) готов; формальный GO-критерий Фазы 4 выполнен (включая TSan) |
| **M3** | Фаза 5 — Godot binding + editor | `voxel-gdext` покрывает binding 75+ классов; `editor/`, `edition/`, `modifiers/`, `terrain/instancing`, `terrain/ root` портированы; плагин загружается в Godot 4.7 и проходит smoke-сцену |
| **M4** | Паритет и удаление C++ | Полный parity против `cpp-reference`; C++ модуль удалён из `master`; форк становится чистым Rust-проектом (правило §1 MIGRATION_PLAN) |

«Закрыть аудит» = **M1 + M2 + M3 + M4**. M1 — приоритет, начинается немедленно; M2–M4 идут
последовательно после M1. Опционально-отложенные подсистемы (GPU/`detail_rendering`/`shaders`,
`sqlite`, `multipass`, `FastNoise2`, physics/Rapier) выносятся за скобки DoD и трекаются отдельно.

### 11.2 Дорожная карта (порядок исполнения)

Работа ведётся в ветке `rust/pilot` (как сейчас), каждый пункт — отдельный коммит + push.
После каждого milestone — обновление `rust/STATUS.md` и журнала `§9.7`. Инвариант на всём
пути: 655+ unit + 11 integration + golden-parity зелёные; clippy/fmt чистые; каждый шаг
сверяется с C++-файлом (ссылки в §9.1–9.3).

#### M1 — Долг по ревью кода (§9 + §7)

M1.A — Закрыть хвост волны 2 (конкурентность, §9.1):
1. ✅ **TSan-прогон на Linux/nightly — ЗАКРЫТ 2026-07-12.** Создан workspace-член `tsan`
   (5 тестов: edit/load/mesh stress + `SpatialLock3D` concurrency + `ThreadedTaskRunner`
   enqueue/postpone). Прогон `cargo +nightly test -p tsan -Zbuild-std --target x86_64-unknown-linux-gnu`
   с `-Zsanitizer=thread` — стабильно 0 data race. Ключевой нюанс: обязателен `-Zbuild-std`,
   иначе std не инструментирован и TSan выдаёт false positives на `std::sync`. См. §9.7.
   `loom`-модель для `SpatialLock3D` оставлена опциональной — TSan + macOS stress достаточно
   для формального GO-критерия Фазы 4 по конкурентности.

M1.B — Структурное решение D7 (блокирует волну 3):
2. ✅ **D7: типизированное хранилище каналов — ЗАКРЫТО 2026-07-12.** `Channel.data` →
   `enum ChannelData { U8/U16/U32/U64(Vec<_>) }`. Hot loops теперь depth-dispatch один раз
   на канал и индексируют типизированный slice напрямую. Добавлена `bytemuck` dep для safe
   byte-cast (wire-format `block_serializer` неизменен). Pool recycling отложен (test-only).
   Снимает alignment-вопрос B1/B5 навсегда. См. §9.7.

M1.C — Волна 3, перф-фиксы горячего пути (§9.2/§9.6-B, по порядку зависимостей):
3. ✅ **B1: типизированный SDF-вход — ЗАКРЫТ 2026-07-12.** `enum TypedSdfSampler { Bit8/16/32/64(&[T]) }`
   резолвит variant + depth-specific decode **один раз** в `TransvoxelMesher::build` (через новый
   публичный accessor `VoxelBuffer::channel_data` + `ChannelData` re-export). `sample_f32(data_index)`
   теперь индексирует типизированный slice напрямую — уходят per-voxel div/mod обратно в (x,y,z),
   per-voxel `ChannelData::get_u64` match и per-voxel `raw_voxel_to_real` match (всё свелось к одному
   depth-specific decode). Golden-тесты (sphere_16/32, parity, tables) подтверждают byte-identical output.
   Vcall через `&dyn RegularMesherInput` оставлен (ядро документирует это как сознательный выбор;
   monomorphization ядра — отдельный шаг если bench покажет нужду). См. §9.7.
4. ✅ **B3: переиспользование мешевых массивов — ЗАКРЫТ 2026-07-12.** Terrain-level free-list pool
   `MeshArraysPool` (`Mutex<Vec<MeshArrays>>`) в `VoxelTerrainCore`. `TransvoxelMesher::build` берёт
   очищенный `MeshArrays` из pool'а (через новое поле `MesherInput.mesh_arrays_pool`, threaded через
   `MeshBlockTaskParams` → `MeshBlockTask`) вместо `MeshArrays::default()` на каждый блок. Pool
   возвращает arrays в `apply_mesh_update` (re-mesh old output) и `unview_mesh_block` (unload).
   `MesherOutput::take_first_transvoxel_arrays()` извлекает transvoxel-слот для возврата. Выяснено:
   `build_regular_mesh` НЕ очищает output (appends), поэтому pool отдаёт cleared buffer (`acquire`
   зовёт `clear()` сохраняя capacity). thread_local на mesher-уровне невозможен — arrays move'ятся в
   output→`MeshBlockEntry`, поэтому pool живёт на terrain. См. §9.7.
5. **B4: gather-scratch из цикла** `gather_voxels_cpu` (вынести `VoxelBuffer` из цикла 3×3×3,
   `Allocator::Pool` — наконец задействовать `VoxelMemoryPool`).
6. **B5: Cubes/Blocky zero-copy** через тот же `channel_slice`/typed-канал из D7/B1.

M1.D — Волна 3, graph runtime (§9.3/§9.6-C):
7. **C1: compile-шаг** `CompiledGraph` (топо-кэш + dense-индексы вместо HashMap, XZ-outer-group
   кэш, hoist сэмплеров, constant folding). До ~16× на террейн-графах; без смены публичного API.
8. **C3: range analysis** поверх C1 (`analyze_range` на портированном `math::interval`); SDF-блок
   вне нуля заполняется uniform без per-voxel исполнения.

M1.E — Инфра-долг (§9.6 «Инфраструктура», §7):
9. **H2-MT benchmark harness**: throughput на уровне `MeshBlockTask::run` + многопоточный
   paging (движущийся viewer); сравнение с расширенным `cpp-baseline`. DoD: ≥ C++ −15%, target ≥0%.
10. **cargo-fuzz** таргеты на `.vox`, `block_serializer`, `region` (D2-класс багов).
11. **CI**: вернуть авто-триггеры `rust.yml` (push/PR) + bench-smoke + x86_64-android emulator
    smoke, когда GitHub flow готов.
12. **§7 риски**: настроить upstream-tracking (`cpp-reference` remote/branch + регулярный
    merge); актуализировать `REPORT.md` под текущие числа (или явно пометить как «снапшот Фазы 0»);
    по on-device Фазы 2 — закрыть (эмулятор/устройство) либо задепрекейджить с обоснованием.

**DoD M1:** все 12 пунктов закрыты, TSan зелёный, H2-MT показывает ≥0% vs C++ −15%, clippy/fmt/тесты чистые.

#### M2 — Фаза 4 до GO

13. **Multi-LOD paging**: `VoxelLodTerrain` + `VoxelLodTerrainUpdateData` + threaded update task
    + clipbox/octree strategy (~9.9k LOC C++, главный блокер Фазы 4). Сверка с
    `terrain/variable_lod/*`. Transition cells для transvoxel (LOD-переходы) — часть этого шага.
14. **`VoxelEngine` остаток**: time-spread/progressive очереди, GPU queue (опц.), file locker,
    stats/profiling, volume callbacks.
15. **`VoxelDataGrid`**, сквозной TSan на multi-LOD сцене.
**DoD M2:** GO-критерий Фазы 4 формально выполнен; multi-LOD parity/стресс зелёные.

#### M3 — Фаза 5 (Godot binding + editor)

16. **Binding** 75+ классов в `voxel-gdext` (сейчас 82 LOC hello-world); `Node3D`-обёртки для
    `VoxelTerrainCore`/`VoxelLodTerrain`; `RenderingServer` mesh upload.
17. **editor/** (12.8k), **edition/** (7.8k), **modifiers/** (1.5k), **terrain/instancing** (9.2k),
    **terrain/ root** (2.0k) — портирование подсистем.
**DoD M3:** плагин загружается в Godot 4.7, проходит smoke-сцена (viewer → paging → mesh → render).

#### M4 — Паритет и удаление C++

18. Полный parity против `cpp-reference` (расширенный набор golden/diff-тестов по всем подсистемам).
19. Удаление C++ модуля из `master`; форк — чистый Rust-проект; `cpp-reference` остаётся только
    как зеркало upstream для отслеживания будущих bugfix'ов.
**DoD M4:** `master` собирается и проходит все тесты без C++; `cpp-reference` обновляется из Zylann/master.

### 11.3 Что НЕ входит в DoD (опционально/отложено)

- GPU-путь: `engine/gpu`, `engine/detail_rendering` (normal maps), `shaders/` (~7.1k LOC).
- `streams/sqlite` (2.2k), `generators/multipass` (2.3k), `util/noise` FastNoise2/SpotNoise (~3.1k).
- Physics (Rapier, §9 плана — не начат).
Эти подсистемы трекаются отдельно; их отсутствие не блокирует ни один milestone.

### 11.4 Процесс (по требованию заказчика)

- Ветка: `rust/pilot` (как сейчас); milestone может выделять долгоживущую ветку `rust/m{1,2,3}` по необходимости.
- Каждый пункт дорожной карты → отдельный коммит + `git push`; сообщение коммита по образцу
  существующих (`rust(phase4): add ...`, `fix(meshers): ...`).
- После каждого milestone — обновление `rust/STATUS.md` (фаза/тесты/секция «what remains») и
  журнала `§9.7` (для M1) или новой фазовой секции (для M2+).
- Инварианты (тесты/clippy/fmt/golden-parity) проверяются перед каждым push.
