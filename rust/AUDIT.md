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
стаб SpatialLock3D) проверены и совпадают с кодом. Расхождений «документация говорит одно,
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

## 2. Проверенные факты (прогон 2026-07-06, macOS)

| Проверка | Заявлено | Фактически |
|---|---|---|
| `cargo test -p voxel-core` | 635 unit + 10 integration + 1 doc-test | ✅ 635 unit + 10 integration (5 e2e + 2 parity + 2 sphere + 1 tables) + 1 doc-test, 0 failed, 1 ignored (diagnostic dump) |
| `cargo clippy --workspace --all-targets` | чистый | ✅ чистый |
| `cargo fmt --check` | чистый | ✅ чистый |
| `cargo build -p voxel-gdext` | собирается | ✅ собирается |
| H1 mesh parity vs C++ goldens | pass (sphere_16: 888 verts/3912 idx; sphere_32: 3696/18600) | ✅ `transvoxel_parity` проходит против закоммиченных C++ goldens |
| Transvoxel-таблицы | byte-identical upstream C++ | ✅ `transvoxel_tables_parity` проходит |
| `ThreadedTaskRunner` | реальные потоки | ✅ `std::thread::spawn` в worker-пуле (`tasks/threaded_task_runner.rs`) |
| `SpatialLock3D` | no-op стаб | ✅ подтверждено (`thread/mod.rs`), эксклюзивность через `&mut self` |
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
| util/thread | Mutex/RwLock/Semaphore, реальный task runner | настоящий `SpatialLock3D` (стаб), `SpatialLock2D`, ThreadSanitizer end-to-end |
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
   TSan не запускался, `SpatialLock3D` — стаб, конкурентный edit+mesh путь ещё не существует.
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

- **Фаза 4 остаток**: variable_lod (9,9k) + VoxelEngine остаток + VoxelDataGrid + real SpatialLock3D + TSan — крупнейший кусок: multi-LOD оркестратор.
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
3. **`VoxelData` за одним глобальным `Mutex`** — вместо C++ per-LOD `RWLock` + `SpatialLock3D`
   (регионная эксклюзивность). Даже чистые чтения двух mesh-тасков в разных регионах сериализуются.
   Примечательно: собственные `thread::RwLock` и каркас `SpatialLock3D` в крейте уже есть — просто не подключены.
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
- `notify_all()` на каждый enqueue/завершение — thundering herd; C++ будит ровно один поток semaphore-постом.
  Наивный `notify_one` некорректен (та же condvar обслуживает `wait_for_all_tasks`) — нужен отдельный
  сигнал работы (готовый `thread::Semaphore` лежит рядом неиспользованным).
- `pick_prioritized_task` — безусловный O(n) скан под общим замком на каждый pick; C++ разводит
  enqueue/pick по разным замкам (staging queue) и сортирует по 32ms-окну с O(1) pop.
- `VoxelTerrainCore` энкьюит таски поштучно в цикле, хотя `enqueue_many` существует (`voxel_terrain_core.rs:475,506`).
- `process()` дважды блокирующе ждёт `wait_for_all_tasks()` за тик — задокументированное временное
  упрощение, но реальный Godot-биндинг поверх него получит стоп-кадры вместо стриминга.

**storage:**
- Нет `#[inline]` на `get_voxel`/`set_voxel`/`get_voxel_f`/`set_voxel_f` (×2: VoxelBuffer и VoxelDataMap) —
  самые частовызываемые функции модуля не инлайнятся кросс-крейтно без LTO.
- `fill_area` пересчитывает полный ZXY-индекс на каждый воксель (`voxel_buffer.rs:409-446`), хотя
  `funcs::fill_3d_region_zxy` в том же крейте правильно выносит базу строки из внутреннего цикла.
- Per-voxel depth-dispatch в `downscale_to`/`paste_masked*` — унаследован от «неоптимизированных» C++ путей,
  но Rust-эквивалента оптимизированного `write_box_template` (dispatch once) нет вовсе.

**streams/io/format:**
- **`RegionFile` deferred header write после аудита закрыт 2026-07-06**: в исходном состоянии
  `save_block` переписывал весь header+LUT (~16 KiB) на диск на каждое сохранение блока
  (`region_file.rs:547`); C++ держит `_header_modified` и пишет header один раз в `flush()/close()`.
  N сохранений = N перезаписей заголовка — прямое усиление I/O при save-штормах.
- **`.vox`-парсер не проверяет отрицательные размеры модели** (`parser.rs:287`): `0xFFFFFFFF` → `-1i32`
  проходит проверку `> MAX_MODEL_SIZE`, затем `volume_u64()` sign-extend'ит в ~1.8×10¹⁹ и `vec![0u8; ...]`
  при `panic=abort` **гарантированно роняет процесс** на битом/вредоносном файле. Фикс — проверка `0..=MAX`.
- `VoxelBuffer::create()` безусловно сбрасывает кастомные channel depths — расходится и с C++
  (сохраняет при `format==null`), и с собственным doc-комментарием; сегодня замаскировано call-site'ами.
- `unsafe` raw-pointer aliasing в `blocky/bake.rs:573-592` — обходит borrow checker вручную; путь
  одноразовый (bake), но хрупкий; заменяется безопасным compute-then-assign.

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
> без внешних generator/mesher mutex. Оставшаяся часть #1: `VoxelData` per-LOD locking.

| # | Что | Эффект | Стоимость |
|---|---|---|---|
| 1 | `generate_block(&self)` / `build(&self)` + `Arc<dyn>` вместо `Arc<Mutex<Box<dyn>>>`; `VoxelData` → per-LOD RwLock + реальный `SpatialLock3D` | разблокирует всю многопоточность; чем позже — тем дороже (breaking trait change) | средняя, **делать до multi-LOD и Фазы 5** |
| 2 | ABBA-дедлок Data↔Generator | устраняется п.1; иначе — фиксированный порядок замков | ~0 после п.1 |
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

**A3. VoxelData → per-LOD `RwLock` + реальный `SpatialLock3D` — рекомендуется.**

```rust
struct Lod {
    map: thread::RwLock<VoxelDataMap>,   // блок-карта: читатели параллельны
    spatial_lock: SpatialLock3D,         // регионная эксклюзивность для вокселей
}
pub struct VoxelData { lods: Box<[Lod]>, /* bounds, format, ... — иммутабельные */ }
```

Контракт как в C++ (`voxel_data.h:197-235`): mesh-gather берёт `map.read()` + `spatial_lock.read(box)`;
редактирование — `map.read()` + `spatial_lock.write(box)`; добавление/удаление блоков — `map.write()`.
Реальный `SpatialLock3D` — это `Mutex<Vec<(BoxBounds3i, Mode)>>` + `Condvar` (ждать, пока нет
пересечений), ~150 LOC; API-каркас в `thread/mod.rs` уже совпадает по форме, `thread::RwLock` готов.

*Альтернативы (отвергнуты):* шардированный map (DashMap-стиль) — новая зависимость и другая
семантика блокировок, теряем сверяемость с C++; actor-модель (один поток-владелец + каналы) —
убивает параллельные чтения и требует пере-дизайна всех call-site'ов.

*Миграция и подводные камни A3 (дополнено вторым проходом):*
- У потребителей `Arc<Mutex<VoxelData>>` → `type SharedVoxelData = Arc<VoxelData>` с внутренними
  замками (как C++ `std::shared_ptr<VoxelData>`); все `data.lock()` в `MeshBlockTask` /
  `VoxelTerrainCore` / `LoadBlockForTerrainTask` заменяются методами `VoxelData`, берущими
  нужный scoped-замок внутри. Самый объёмный (но механический) кусок волны 2.
- Поля-настройки (`generator`, `stream`, `bounds`, `streaming_enabled`, ...) — в
  `RwLock<Settings>` (зеркало C++ `_settings_lock`): иначе `set_generator`/`set_bounds`
  потребуют `&mut VoxelData`, которого при `Arc<VoxelData>` больше нет. Это же отвечает
  на вопрос A1, где живёт `Arc<dyn VoxelGenerator>`.
- Межлодовый порядок: `update_lods` держит замки двух LOD одновременно (src→dst) —
  зафиксировать правило «замки LOD берутся строго по возрастанию индекса», иначе появится
  новый ABBA между LOD-каскадом и редактированием.

**A4. Порядок замков.** После A1 замок генератора исчезает — ABBA-дедлок уходит сам.
Дополнительно зафиксировать правило в doc-комментарии `VoxelData`: *замок данных никогда
не держится через вызов generator/mesher/stream* (сейчас нарушается в `LoadBlockForTerrainTask`).

**A5. Task runner (после A1-A3, отдельными коммитами):**
- будить воркеров через готовый `thread::Semaphore` (post на каждый enqueue, как C++
  `_tasks_semaphore`); существующую condvar оставить только для `wait_for_all_tasks`;
- staging-очередь на отдельном замке для enqueue + сортировка раз в 32ms-окно + O(1) pop
  с конца (зеркало `threaded_task_runner.cpp:104-270`) вместо O(n)-скана на каждый pick;
- `VoxelTerrainCore`: собирать таски в `Vec` и звать `enqueue_many` (уже существует);
- `process()`: заменить два блокирующих `wait_for_all_tasks()` на неблокирующий drain
  завершённых тасков (+ бюджет времени на тик) — обязательное условие для Phase 5 биндинга.

**Валидация волны A:** stress-тест (8 потоков: параллельные mesh + edit + load на общей
`VoxelData`, счётчики целостности), прогон под TSan (nightly `-Zsanitizer=thread`, на Linux-хосте),
опционально loom для `SpatialLock3D`. Это же закрывает GO-критерий Фазы 4.

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
| D2 | `.vox`: `(0..=MAX_MODEL_SIZE).contains(&size.{x,y,z})` | закрывает abort на отрицательных размерах; заодно строже C++ |
| D3 | `VoxelBuffer::create(size, format: Option<&VoxelFormat>)` | либо сохранять текущие depth при `None` (C++ семантика) + поправить doc-комментарий |
| D4 | `#[inline]` на `get_voxel`/`set_voxel`/`get_voxel_f`/`set_voxel_f` ×2 типа; `fill_area` — база строки за внутренний цикл (образец: свой же `fill_3d_region_zxy`); depth-hoisted helper для `downscale_to`/`paste_masked*` (аналог C++ `write_box_template`) | |
| D5 | `HeightmapNoise::curve: Option<Arc<Curve>>` | ✅ закрыто 2026-07-06: клон 256×f32 на блок заменён на O(1) refcount (образец — graph-узел Curve) |
| D6 | `blocky/bake.rs`: compute-then-assign вместо `unsafe` raw-pointer aliasing | считаем cutout-данные в локальный `Vec` под shared borrow, присваиваем после |
| D7 | **(структурная опция)** типизированное хранилище каналов: `enum ChannelData { U8(Vec<u8>), U16(Vec<u16>), U32(Vec<u32>), U64(Vec<u64>) }` | снимает alignment-вопрос B1/B5 навсегда, делает depth-dispatch естественным (match один раз → типизированный слайс) и бесплатно даёт «write_box_template»-аналог из D4; цена — рефакторинг `voxel_buffer.rs` + сериализатора (typed→bytes каст безопасен всегда). Решить до волны 3 |

#### Порядок внедрения

Дорожки A (конкурентность), B (мешинг), C (граф), D (точечные) почти независимы — под модель
«AI 24/7» их можно вести параллельными сессиями. Явные связки: B1/B5 требуют решения по
хранению каналов (D7 или выровненный пул — см. ⚠️ в B1); A2-вариант-3 включает мини-правку
`tasks`; B3 естественно делается вместе с A2.

**Волна 1 — дешёвое и разблокирующее (каждый пункт — отдельный коммит):**
A1 (генератор `&self`) → A2 (мешер `&self` + scratch) → A4 (правило замков).
Уже после этой волны mesh-build'ы и генерация исполняются параллельно (глобальный замок
`VoxelData` остаётся только на gather — короткая секция).
**DoD:** новый тест «N воркеров мешат M блоков» показывает масштабирование по потокам
(время ~1/N, загрузка >1 ядра); 640+10 тестов и golden-парити зелёные.

**Волна 2 — конкурентность до конца:**
A3 (`Arc<VoxelData>` + per-LOD RwLock + реальный SpatialLock3D) → A5 (semaphore + staging +
неблокирующий drain в `process()`).
**DoD:** stress-тест (8 потоков: mesh + edit + load на общей `VoxelData`) стабилен; TSan-прогон
чист (nightly `-Zsanitizer=thread`, Linux-хост); GO-критерий Фазы 4 закрыт формально.

**Волна 3 — производительность:**
решение по D7 → B1 → B3/B4/B5 → C1 → C3 → D3/D4/D6.
**DoD:** новый бенч **H2-MT** — throughput на уровне `MeshBlockTask::run` + многопоточный
paging-сценарий (движущийся viewer), сравнение с расширенным `cpp-baseline`; критерий прежний:
не хуже C++ −15%, target ≥0%.

**Инфраструктура (найдено вторым проходом, вне волн):**
- **CI для `rust/` не существует**: все 9 workflow в `.github/workflows/` собирают только C++
  (ни один не знает про cargo), метрика плана §6 «build time CI <15 мин» не отслеживается.
  Добавить workflow: `cargo test/clippy/fmt` + Android cross-build smoke (скрипт уже есть) +
  бенч-смоук. Дёшево и сразу защищает все волны от регрессий.
- **Бенч-харнесс H2-MT** (см. DoD волны 3) — без него результаты волн неизмеримы: текущий бенч
  меряет только ядро мешера в один поток.
- **cargo-fuzz таргеты на парсеры** (`.vox`, `block_serializer`, `region`): C++-сторона уже
  фаззится (`fuzzer.yml`), Rust-парсеры — нет; баг D2 — ровно тот класс, который находит фаззер.

Инварианты на всём протяжении: 640 unit + 10 integration + golden-парити остаются зелёными;
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

Остаток пункта #1: `VoxelData` per-LOD `RwLock`/real `SpatialLock3D`.
ABBA-риск с внешним generator/mesher lock снят, но правило “не держать data lock через
generator/mesher/stream” ещё нужно закрепить и довести в A4/A3. Переиспользование
`MeshArrays`/`MesherOutput` остаётся отдельной perf-частью B3.

## 10. Вывод

Миграция идёт дисциплинированно: каждая фаза закрывается тестами, документация точна,
код без заглушек-паник, parity подтверждён исполняемыми тестами, зависимость от C-кода
минимальна (важно для Android/WASM). Ревью кода подтверждает: **алгоритмическая корректность
и идиоматика на высоте** (порт не механический: Option вместо сентинелей, RAII-пул, точные
byte-parity тесты), но **два системных долга** требуют внимания до продолжения Фазы 4/5:

1. **Конкурентная модель** (§9.1): trait-level сериализация генерации и мешинга уже снята
   после аудита, но `VoxelData` всё ещё стоит за одним глобальным mutex. Пул потоков
   начнёт масштабироваться полноценно только после per-LOD `RwLock`/real `SpatialLock3D`
   и закрепления порядка замков.
2. **Горячий путь мешинга** (§9.2): адаптерный слой вернул абстракционные издержки, которые C++
   целенаправленно устранял; заявление H2 о 1.5× преимуществе не распространяется на end-to-end конвейер.

План действий — три волны из §9.6: (1) остаток волны 1 — A4,
(2) волна 2 — per-LOD RwLock + реальный SpatialLock3D + TSan/stress
(закрывает GO-критерий Фазы 4), (3) волна 3 — перф-фиксы горячего пути и graph runtime
с перемером H2 end-to-end. Параллельно: настроить upstream-tracking (`cpp-reference`) и
CI для `rust/` — сейчас Rust не собирается ни одним workflow.
Multi-LOD paging начинать после волны 2 — уже на исправленной threading-модели.
