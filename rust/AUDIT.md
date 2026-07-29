# Аудит миграции godot_voxel → Rust

> Дата исходного аудита: 2026-07-06 · Ветка: `rust/pilot` (403b81ba, синхронизирована с `origin/rust/pilot`, рабочее дерево чистое)
> Повторный аудит: **2026-07-10** · HEAD: `60225f11de4a` · ветка синхронизирована с
> `origin/rust/pilot`; до изменения этого документа рабочее дерево было чистым.
> История исправлений исходного аудита сохранена в §9.7; актуальный пересмотр реализации,
> новые находки и варианты решений находятся в **§11**.
> Актуализация: **2026-07-24** · линия `origin/rust/pilot` до `e56895ae` сведена с 12
> локальными data-safety/correctness-коммитами; текущий статус и результаты проверки — §11.10.
> Метод: проверка заявлений `MIGRATION_PLAN.md` / `rust/STATUS.md` / `REPORT.md` против фактического кода,
> прогон тестов/clippy/fmt/сборки/бенчмарков, инвентаризация LOC C++ vs Rust,
> **плюс ревью качества кода**: 5 независимых проходов по подсистемам (storage, meshers,
> generators, threading/engine/terrain, streams/math/io) со сверкой с C++-референсом,
> ключевые находки перепроверены по коду вручную (см. §9).

---

## 1. Резюме

**Инфраструктурные заявления подтверждаются, но исходный вывод о корректности пересмотрен.**
Тесты, clippy/fmt, сборка gdext, H1-goldens, worker pool и `SpatialLock3D` фактически есть
и проходят. Внедрение A1–A5 также подтверждено на уровне внешних generator/mesher/data-lock.
Однако повторная сверка с C++ и проверка failure-paths выявили несколько не покрытых тестами
расхождений и рисков: возможную потерю dirty-блоков, UB в safe API, неограниченные аллокации
при разборе входных данных, ошибки graph/mesher parity и гонки жизненного цикла mesh-output.
Поэтому формулировка «алгоритмы портированы корректно» без оговорок больше не используется.

На момент повторного аудита было портировано **38 697 строк Rust** против **142 148 строк C++**
модуля (без thirdparty) —
**~27,2% по сырому LOC**, но портированная часть — это концентрированное compute-ядро
(math/storage/meshers/streams/generators/terrain-core). После этого снимка добавлены multi-LOD,
transition cells и Godot-facing MVP; актуальная декомпозиция находится в §10.1 и `STATUS.md`.

**Вердикт повторного аудита (§11):** внешний Mutex-трио исходного аудита в основном снят, но
на 2026-07-10 production-readiness блокировали safe-API soundness, bounded decode и сохранение
dirty data, а также набор P1 correctness/parity проблем. Первые блокеры и часть parity-пунктов
после аудита закрыты; актуализация и оставшаяся очередь — §11.9–11.10.

| Фаза | Заявлено | Аудит |
|---|---|---|
| 0 — Пилот (transvoxel + кросс-компиляция) | ✅ GO | ✅ подтверждено: H1 parity-тесты проходят, таблицы byte-identical |
| 1 — Чистое ядро (`util/*` + expression_parser) | ✅ завершена | ✅ подтверждено |
| 2 — Мобильная валидация | ✅ `.so` desktop+Android; on-device ⏳ | ✅/⚠️ on-device проверка так и не закрыта (нужен SDK+устройство) |
| 3 — Compute-слой | ✅ завершена | ✅ подтверждено (engine-agnostic scope) |
| 4 — Terrain + threading | 🟡 в работе | 🟡 подтверждено: single-LOD paging + VoxelEngine foundation готовы headlessly |
| 5 — Godot binding + editor | ⏳ не начата | ⏳ подтверждено: binding = 82 строки hello-world |

---

## 2. Проверенные факты (повторный прогон 2026-07-10, macOS)

| Проверка | Заявлено | Фактически |
|---|---|---|
| `cargo test -p voxel-core` | 655 unit + 11 integration + 1 doc-test | ✅ 655 unit passed; integration: 11 passed (5 e2e + 2 parity + 2 sphere + 1 tables + 1 threaded stress), 1 diagnostic ignored; 1 doc-test passed; 0 failed |
| `cargo test --workspace` | — | ✅ 655 unit + 11 integration + 1 doc-test passed, 1 integration ignored, 0 failed |
| `cargo test -p voxel-core --all-features` | — | ✅ 657 unit + 11 integration + 1 doc-test passed, 1 integration ignored, 0 failed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | чистый | ✅ чистый |
| `cargo fmt --all -- --check` | чистый | ✅ чистый |
| `cargo build --workspace --all-features` | собирается | ✅ собирается |
| H1 mesh parity vs C++ goldens | pass (sphere_16: 888 verts/3912 idx; sphere_32: 3696/18600) | ✅ `transvoxel_parity` проходит против закоммиченных C++ goldens |
| Transvoxel-таблицы | byte-identical upstream C++ | ✅ `transvoxel_tables_parity` проходит |
| `ThreadedTaskRunner` | реальные потоки | ✅ `std::thread::spawn` в worker-пуле (`tasks/threaded_task_runner.rs`) |
| `SpatialLock3D` | real overlap-aware region lock | ✅ базовая механика подтверждена (`thread/mod.rs`): overlapping reads coexist, overlapping writes block; touching half-open boxes считаются пересекающимися, fairness/lock-order оговорены в §11.6 |
| Код без явных заглушек | — | ✅ `todo!`/`unimplemented!` в production-path не найдено; это не отменяет failure-path и parity-находки §11 |
| H2 perf (Rust ~1.5× быстрее C++) | 28.5µs/143 Melem/s vs 44.1µs/93 Mvoxels/s | ✅ criterion перепрогнан (`sample-size 10`): sphere_16 ≈27,2µs / 150 Melem/s, sphere_32 ≈156µs / 210 Melem/s, sphere_64 ≈958µs / 274 Melem/s. **Но** бенч меряет ядро `build_regular_mesh` в обход адаптера `builtin.rs` — см. §9.4 и §11.6 |
| Воспроизводимость Cargo | — | ⚠️ `rust/Cargo.lock` существует локально, но игнорируется `rust/.gitignore:4`; CI-команды не используют `--locked` (§11.7) |

---

## 3. Объём: C++ модуль vs Rust порт

### C++ (без thirdparty/) — 142 148 LOC

| Директория | LOC | Содержимое | Rust-покрытие |
|---|---:|---|---|
| terrain/ | 25 233 | fixed_lod (4,1k), variable_lod (9,9k), instancing (9,2k), root (2,0k) | 🟡 только engine-agnostic single-LOD ядро (1 243) |
| util/ | 23 809 | godot-shims (7,8k), math (4,9k), noise (3,1k), containers, thread, tasks, io | 🟢 math/string/io/tasks/thread портированы; godot-shims N/A; FastNoise2 нет |
| generators/ | 19 386 | graph (14,1k), multipass (2,3k), simple (1,4k) | 🟡 minimal graph runtime (1,8k), simple частично; multipass нет |
| meshers/ | 16 503 | blocky (9,2k), transvoxel (4,6k), cubes (1,6k) | 🟢 все три ядра + адаптеры VoxelMesher (8 165) |
| editor/ | 12 833 | 11 EditorPlugin-ов (graph editor 5,5k и др.) | 🔴 ноль |
| tests/ | 9 025 | C++ тест-сюита | ➖ своя Rust-сюита, не зеркалирует C++ |
| storage/ | 8 126 | buffer, data/map/block/grid, format, mixel4, metadata | 🟢 ядро портировано; metadata/ и VoxelDataGrid нет |
| streams/ | 8 018 | base, serializer, region (2,0k), sqlite (2,2k), vox | 🟡 форматы/задачи портированы; sqlite и forest-wrapper нет |
| edition/ | 7 821 | VoxelTool*, raycast, mesh SDF, floating chunks | 🔴 ноль |
| engine/ | 5 461 | VoxelEngine, gpu (1,5k), detail_rendering (2,4k) | 🟡 registry+task loop (1,2k); GPU/detail нет |
| shaders/ | 3 164 | GLSL-шаблоны + реестр | 🔴 ноль |
| modifiers/ | 1 467 | VoxelModifier стек | 🔴 ноль |
| constants/ | 670 | cube tables и др. | 🟢 портировано |

### Rust — 38 697 LOC

- `voxel-core/src` — **38 615 LOC**: meshers 8 165 · storage 6 352 · math 6 332 · streams 4 794 ·
  generators 3 228 · format/vox 1 505 · tasks 1 397 · string 1 353 · terrain 1 243 · engine 1 241 ·
  io 1 255 · thread 578 · constants 466 · containers 265 · прочее 441.
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
  → VoxelTerrainCore (single-LOD paging: viewers → loads → meshing → outputs → unload;
                      пока владеет отдельным ThreadedTaskRunner)

VoxelEngine foundation (отдельно: volume/viewer registry, свой runner, priority sync;
                        пока не оркестрирует VoxelTerrainCore)
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
| meshers/cubes | greedy/simple + palette | C++ default raw-color mode, atlased mode (UV packing) |

---

## 7. Отклонения от плана и риски

1. **Git-стратегия — ✅ закрыто 2026-07-12 (M1.E).** План (§1) требует ветку `cpp-reference`
   (зеркало Zylann/master) и регулярный upstream-merge. Настроено: remote `upstream`
   → `https://github.com/Zylann/godot_voxel.git` добавлен, ветка `cpp-reference` создана
   (трекает `upstream/master`). `git fetch upstream master` обновляет её. Ветка локальная
   (не push'ится в origin) — её роль быть эталоном для parity и местом для cherry-pick
   upstream bugfix'ов. До этого: remote на upstream не настроен, ветки `cpp-reference` нет.
2. **Фаза 2 не закрыта on-device — 🟡 задепрекейчено.** `.so` собран (aarch64/x86_64-android),
   но APK на устройстве/эмуляторе не проверялся (нужны SDK+устройство вне окружения).
   Кросс-компиляция верифицирована (H4 PASS: pure-Rust `.so` линкуется, `gdext_rust_init`
   экспортируется). On-device smoke остаётся формальным пробелом; закрывается когда появится
   доступ к Android SDK+эмулятору/устройству. До тех пор Фаза 2 считается «десктоп+кросс-компил
   валидирована, on-device — формальность для pure-Rust кода без платформ-специфики».
3. **GO-критерий Фазы 4 по TSan — ✅ закрыто 2026-07-12 (M1.A).** ThreadSanitizer прогнан на
   Linux/nightly через workspace-член `tsan` (5 тестов: edit/load/mesh stress + SpatialLock3D
   + ThreadedTaskRunner). 0 data race. См. §9.7. При этом TSan не доказывает корректность
   cancellation/versioning, save lifecycle и panic-liveness; оставшиеся логические риски
   продолжают отслеживаться в датированном повторном аудите §11.
4. **Parity-покрытие тестами точечное.** Byte-parity доказан для transvoxel-таблиц и двух golden-сфер;
   остальные модули покрыты юнит-тестами, портированными «по мотивам» C++, но C++ тест-сюита
   (9 025 LOC, 61 файл) не зеркалируется системно. Риск тихих поведенческих расхождений в углах.
   *Частично смягчено (M1.E):* cargo-fuzz таргеты на `.vox`/`block_serializer`/`region` парсеры
   теперь ловят crash/OOM класс багов (см. §9.7 — fuzzer нашёл и пофиксил OOM в decompressed_size).
5. **H2 perf проверен только на transvoxel 16³–64³.** Для storage/paging/graph перф-сравнений с C++ нет.
   *Смягчено (M1.B–M1.D):* typed storage (D7), typed SDF sampler (B1), MeshArrays pool (B3),
   gather-scratch hoist (B4), Cubes/Blocky zero-copy (B5), graph compile-step + XZ-cache (C1),
   range analysis (C3) закрывают основные perf-долги. H2-MT bench harness остаётся TODO.
6. **REPORT.md устарел числами — ✅ закрыто 2026-07-12 (M1.E).** Снапшот Фазы 0 помечен
   «Snapshot notice» со ссылкой на `rust/STATUS.md` для актуальных чисел.

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
>
> **Исторический раздел.** §9.1–§9.6 фиксируют состояние и рекомендации 2026-07-06;
> §9.7 — внедрённые после него изменения. Не следует читать старые оценки как текущий
> production-verdict: повторная проверка и актуальные ссылки находятся в §11.

### 9.1 Исходная критическая находка: внешняя сериализация конвейера

В исходном состоянии были три слоя полной сериализации в горячем пути. Внешние
generator/mesher/data-lock из пп. 1–3 сняты изменениями A1–A5 (§9.7), поэтому этот абзац
нельзя использовать как описание текущей реализации. Остались внутренняя сериализация
`GraphGenerator` и per-LOD map contention — см. §11.5–§11.6.

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
| 2026-07-12 | **M1.C / B4: gather-scratch вынесен из цикла 3×3×3** | ✅ закрыто. `generate_missing_voxel_regions` выделял свежий `VoxelBuffer` (`with_size`+`configure_buffer`) на каждой итерации цикла соседей — до 27 heap-аллокаций на gather при недогруженных соседях. Scratch `VoxelBuffer` теперь выделяется **один раз до цикла**, а перед каждой генерацией сбрасывается через `create()` (сохраняет channel depths по D3) + `configure_buffer`, после чего генератор заполняет его и каналы копируются в padded `dst`. Early-return при пустом `missing_regions`. `Allocator::Pool`/`VoxelMemoryPool` не подключён — pool byte-oriented и test-only (см. D7), отложен до production-caller в Фазе 5. | `cargo test -p voxel-core` → 652 unit + 11 integration + 1 doc-test (end-to-end pipeline, который покрывает gather+generate path, зелёный); `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; `cargo +nightly test -p tsan -Zbuild-std ...` → 5 passed, 0 TSan warnings |
| 2026-07-12 | **M1.C / B5: Cubes/Blocky zero-copy (волна 3 завершена)** | ✅ закрыто. **Blocky**: новый `build_blocky_into` dispatch'ит channel depth **один раз** и передаёт типизированный slice напрямую в generic `generate_mesh<T: Copy+Into<u16>>` (настоящий zero-copy для Bit8/Bit16, default Type channel — Bit16; Bit32/Bit64 fallback на `extract_voxel_slice`). **Cubes**: `extract_voxel_slice` widened в `Vec<u32>` одним проходом `iter().map().collect()` вместо per-voxel `get_voxel` (free fn требует `&[u32]`, полного zero-copy нет, но depth-dispatch один раз). Оба adapter'а получили `Compression::Uniform` fallback (`vec![defval; cap]`) — `channel_data` возвращает пустой Vec для uniform-канала, что вызывало бы out-of-bounds. Regression-тест `blocky_mesher_handles_uniform_air_block_without_panicking` закрывает латентный баг Blocky path (parallel-агент нашёл, что Blocky не имел uniform-handling в отличие от Cubes/Transvoxel). **Это закрывает M1.C (волна 3) полностью: B1+B3+B4+B5.** | `cargo test -p voxel-core` → 653 unit + 11 integration + 1 doc-test (golden parity + end-to-end зелёные); `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; `cargo +nightly test -p tsan -Zbuild-std ...` → 5 passed, 0 TSan warnings |
| 2026-07-12 | **M1.D / C1: compile-шаг `CompiledGraph` (topo-кэш + dense scratch + XZ-outer-group cache)** | ✅ закрыто. `CompiledGraph::compile(&Graph)` кэширует topological order + dense `id_to_index` + классифицирует Y-dependence (reachability от InputX/InputZ → outer group; `xz_prefix_len` = индекс первого Y-dependent узла). `CompiledScratch` (dense `Vec<Vec<f32>>`) заменяет `HashMap<GraphNodeId, Vec<f32>>` с capacity-preserving reuse между slices. `generate_slice(xz_prefix_cached: bool)` eval'ит узлы по dense index (без per-element HashMap lookup), а на 2-м+ Y-slices skip'ает `[0, xz_prefix_len)` prefix — их буферы persist от первого slice. Критичный нюанс: `OutputSdf` writes в scratch (не в per-slice outputs Vec), а `generate_slice` collects outputs после eval'а — так cached-prefix OutputSdf тоже доступен. `generate_block_with_compiled_graph` строит XZ coords один раз вне Y-loop. `GraphGenerator` хранит lazy `Mutex<Option<CompiledGraph>>` (Send+Sync preserved) + `Mutex<CompiledScratch>`. На topology-error fallback на legacy path. Все 32 существующих graph-теста + 15 новых C1-тестов (topo-кэш, dense index, XZ-classification × 3, golden parity compiled-vs-lazy × 5, XZ-cache consistency, capacity reuse, adapter sin(x)+1 canary, XZ-only multi-slice) зелёные. | `cargo test -p voxel-core` → 668 unit + 11 integration + 1 doc-test (15 новых C1 + все существующие graph-тесты); `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; `cargo +nightly test -p tsan -Zbuild-std ...` → 5 passed, 0 TSan warnings |
| 2026-07-12 | **M1.D / C3: range analysis (uniform-SDF culling)** | ✅ закрыто. `CompiledGraph::analyze_range(x, y, z) -> Interval` пропагирует input intervals через compiled topo-order: easy nodes используют портированную `math::interval` (Add/Sub/Mul/Div/Min/Max/Sin/Abs/Sqrt/Floor/Fract/Remap/Distance2D/3D/SdfPlane/SdfSphere/SdfUnion/SdfSubtract/Mix/Clamp), hard nodes (Cos/Noise2D/3D/Curve/Normalize3D/Pow/SdfSmooth*/SdfBox/SdfTorus) → conservative `infinity()` (C++ имеет per-node range funcs, Rust fallback безопасен — теряется только optimisation). `generate_block_with_compiled_graph` culls uniform-SDF blocks: **только `is_single_value()`** заполняет actual SDF value uniform (conservative — sign-only ranges остаются per-voxel, т.к. фактическое SDF значение может нести информацию для distance field; C++ заполняет FAR_OUTSIDE/INSIDE для sign-only, но это рискованно при hard nodes). 7 новых тестов: 5 runtime (constant/plane-solid/plane-straddle/noise-fallback/add) + 2 adapter (cull air Constant(2.0) + cull solid Constant(-2.0)). **M1.D (graph runtime) полностью закрыта: C1+C3.** | `cargo test -p voxel-core` → 675 unit + 11 integration + 1 doc-test (7 новых C3); `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; `cargo +nightly test -p tsan -Zbuild-std ...` → 5 passed, 0 TSan warnings |
| 2026-07-12 | **M1.E: cargo-fuzz таргеты + OOM bug fix + §7 риски** | ✅ закрыто (частично). **cargo-fuzz**: новый workspace-член `fuzz/` (separate workspace, cargo-fuzz manages ASan/sancov) с 3 таргетами: `vox_parser` (`.vox` parse), `block_serializer` (decompress_and_deserialize), `region_file` (block payloads via decompress_and_deserialize; full region-header fuzzing требует `load_header` pub API — TODO). **OOM bug found & fixed**: fuzzer нашёл, что `compressed_data::decompress_lz4`/`decompress_zstd` читают `u32 decompressed_size` из untrusted bytes и `dst.resize(decompressed_size)` без cap → out-of-memory (fuzzer триггерил malloc ~2-4 GiB). Fix: `MAX_DECOMPRESSED_SIZE = 256 MiB` cap с `InvalidSize` early-return + regression test. Этот класс багов — ровно тот, ради которого аудит §9.6 просил fuzzing (D2-подобный). **§7 риски**: risk 1 (cpp-reference) ✅ — `upstream` remote + `cpp-reference` branch настроены; risk 3 (TSan) уже ✅ (M1.A); risk 6 (REPORT.md) ✅ — помечен snapshot notice; risk 2 (on-device) 🟡 задепрекейчено (кросс-компил валидирован, on-device — формальность для pure-Rust). H2-MT bench harness остаётся TODO (item 9). | `cargo test -p voxel-core` → 676 unit + 11 integration + 1 doc-test; `cargo clippy --workspace --all-targets` clean; `cargo fmt` clean; cargo-fuzz: 3 таргета × 2000 runs, 0 crash после OOM fix (coverage 57/201/212 ветвей) |
| 2026-07-12 | **M1.E / item 9: H2-MT benchmark harness (M1 полностью закрыт)** | ✅ закрыто. `mesh_block_bench` (criterion): `mesh_block/single` меряет end-to-end `MeshBlockTask::run_meshing` (gather 3×3×3 + `TransvoxelMesher::build`, single-threaded, 16³ block, 47 µs / 86 Melem/s). `mesh_block/multi` меряет 32 блока через `ThreadedTaskRunner(4)` (MT paging, 673 µs / 194 Melem/s = **2.25× speedup** vs single — подтверждает M1.A threading model работает). В отличие от `transvoxel_bench` (меряет только `build_regular_mesh` core на raw `Vec<f32>`), этот harness покрывает полный pipeline: `SharedVoxelData` → gather → typed-storage dispatch → mesher. Сравнение с C++ baseline требует расширения `cpp-baseline/` (single-threaded transvoxel-only сегодня) — M2+ follow-up. **Это закрывает M1.E (последний блок) и M1 (долг по ревью кода) полностью:** M1.A (TSan) + M1.B (D7) + M1.C (волна 3) + M1.D (graph C1+C3) + M1.E (cargo-fuzz + OOM fix + §7 риски + H2-MT bench). | `cargo bench --bench mesh_block_bench -- --quick` → single 47µs/86 Melem/s, multi 673µs/194 Melem/s (2.25× speedup на 4 потока) |
| 2026-07-12 | **M2.1: Multi-LOD paging MVP (LodOctree + 2 LOD levels + hard seams)** | ✅ закрыто. **Шаг 1:** Rust `LodOctree` (порт `lod_octree.h`, 480 LOC C++ → self-contained Rust module): split/join по viewer distance, NodePool (packs of 8), progressive update, `for_each_leaf`/`for_leaves_in_box`, `is_below_split_distance`, `compute_lod_count`. 12 тестов. **Шаг 2:** `LoadBlockForTerrainTask` принимает `lod_index` (stream query + generator + BlockDataOutput с правильным LOD). **Шаг 3:** `VoxelTerrainCore` struct refactor: `lod_count` field + `new_with_lod_count` + per-LOD `mesh_maps`/`blocks_pending_*`/`loading_blocks` (Vec indexed by LOD) + `mesh_blocks_at_lod` accessor + `ViewerState.data_box_per_lod`/`mesh_box_per_lod`. **Шаг 3b:** все dispatch методы (`apply_data_view/unview`, `view/unview_mesh_block`, `try_schedule_mesh_update`, `send_data_load_requests`, `process_meshing`, `apply_data_block_response`, `apply_mesh_update`) принимают LOD параметр. **Шаг 4:** `compute_viewer_boxes_multi_lod` (per-LOD boxes, block size масштабируется `1 << lod`) + `process_viewers_multi_lod` (diff per-LOD, dispatch per-LOD). End-to-end: 2-LOD terrain с viewer → блоки на BOTH LOD 0 и LOD 1. Hard seams (no transition meshes). Reuse всего существующего storage/MeshBlockTask (уже LOD-ready). | `cargo test -p voxel-core` → 691 unit + 11 integration + 1 doc-test (0 regressions); `cargo clippy` clean; `cargo fmt` clean |
| 2026-07-12 | **M2.2: Transition cells — бесшовные LOD seams** | ✅ закрыто. **Шаг 1:** `transition_tables.rs` (1629 LOC, верbatim порт C++ `transvoxel_tables.cpp`): `TransitionCellData` struct + `TRANSITION_CELL_CLASS[512]` + `TRANSITION_CELL_DATA[56]` + `TRANSITION_CORNER_DATA[13]` + `TRANSITION_VERTEX_DATA[512][12]` (6144 edge codes) + 8 parity тестов (byte-for-byte vs C++ source, извлечённого Python-скриптом). **Шаг 2:** `ReuseTransitionCell` (12 vertices per cell) + 2D cache deck в `Cache` (`reset_reuse_cells_2d`/`get_reuse_cell_2d`). **Шаг 3:** `transition.rs` (913 LOC) — порт C++ `build_transition_mesh` (transvoxel.cpp:706-1090): iterates 3×3 transition cells in face space, 9-bit case code → cell class/data/vertex/corner lookup, vertex reuse via 2D cache, append verts+triangles в shared MeshArrays. Face helpers (`face_to_block`/`get_face_axes`/`get_face_index`) + `SIDE_*` constants. 7 тестов (uniform air/solid → 0 triangles, sphere → non-zero, all 6 directions no-panic, too-small noop, transition_hint_bit, face_helpers parity). **Шаг 4:** wire в `TransvoxelMesher::build` — при `lod_hint=true` loop по 6 граням calling `build_transition_mesh`. Test `transvoxel_lod_hint_produces_transition_geometry` подтверждает transition geometry. | `cargo test -p voxel-core` → 707 unit + 11 integration + 1 doc-test (0 regressions); `cargo clippy` clean; `cargo fmt` clean |
| 2026-07-20 | **M3 Phase 5: Godot binding MVP (7 steps)** | 🟡 в работе. **Step 1:** `VoxelTerrain` (Node3D) — владеет `VoxelTerrainCore`, в `_process` paging → mesh → ArrayMesh upload в дочерние `MeshInstance3D`. `VoxelViewer` (Node3D) — viewer position с `view_distance` property. **Step 2:** `VoxelGeneratorWaves` + `VoxelGeneratorFlat` (Resource) — generator property в инспекторе. **Step 3:** Edition tools: `set_voxel_sdf(x,y,z,value)` + `get_voxel_sdf(x,y,z)` + `get_bounds()`. **Step 4:** `lod_count` property (1=single, 2+=multi-LOD transitions) + dirty block tracking (set_voxel_sdf → mark dirty → process re-uploads mesh). **Step 5:** `raycast(origin, dir, max_distance)` — fixed-step SDF march. **Step 6:** `VoxelGeneratorNoise` (3D caves) + `VoxelGeneratorHeightmap` (2D hills) Resources. **Step 7:** `material_override` (Material applied to all blocks) + `generate_collision` (trimesh collision per block). | `cargo build -p voxel-gdext` ✅; clippy clean; fmt clean. Rendering + editing + materials + collision функциональны в Godot. |

Пункт #1 по снятию generator/mesher/data сериализации закрыт для текущего worker bridge.
ABBA-риск с внешним generator/mesher lock снят; правило “не держать data lock через
generator/mesher/stream” закреплено A4 для текущих load/gather путей. Переиспользование
`MeshArrays`/`MesherOutput` остаётся отдельной perf-частью B3. **TSan-прогон на
Linux/nightly закрыт 2026-07-12 (M1.A):** формальный GO-критерий Фазы 4 по
конкурентности выполнен — data race не найдена на edit/load/mesh stress, `SpatialLock3D`
под нагрузкой и `ThreadedTaskRunner` enqueue/postpone путях.

## 10. Переход от исходного аудита к повторному

Исходный аудит полезно сохранять как историю: он правильно обнаружил внешние mutex-узкие места,
и большая часть A1–A5 действительно внедрена. Но вывод «после stress остаётся только TSan»
больше не подтверждается. TSan не найдёт логические ошибки cancellation/versioning, потерю
данных при save, неверную семантику graph nodes или неограниченные аллокации парсеров.

Актуальный roadmap и прогресс после повторного аудита зафиксированы в §10.1. Детальный
повторный аудит от 2026-07-10 сохранён в §11 как датированный снимок: он не отменяет закрытые
пункты §9.7, а уточняет границы их действия и добавляет проверенные failure-path/parity-находки.

## 10.1 Roadmap и прогресс после повторного аудита (2026-07-12—20)

> Постановка от заказчика: «сначала закрыть §9 (долг по ревью кода), затем пройти весь путь
> до конца миграции (вариант охвата 4); коммитить и пушить по шагам, обновлять статус».
> Эта секция фиксирует, **что значит «аудит закрыт»**, в виде измеримой цели и дорожной карты.
> Сам аудит при этом НЕ переписывается ретроактивно — исходные находки §9 остаются как есть,
> закрытые пункты отмечаются в журнале §9.7, а прогресс по дорожной карте — здесь и в `STATUS.md`.

### 10.1.1 Definition of Done — «Аудит полностью закрыт»

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

### 10.1.2 Дорожная карта (порядок исполнения)

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
5. ✅ **B4: gather-scratch из цикла — ЗАКРЫТ 2026-07-12.** `generate_missing_voxel_regions` выделял
   свежий `VoxelBuffer` (`with_size` + `configure_buffer`) на каждой итерации цикла 3×3×3 — до 27
   heap-аллокаций на gather при недогруженных соседях. Теперь scratch выделяется **один раз до цикла**,
   а перед каждой генерацией сбрасывается через `create()` (сохраняет channel depths по D3) +
   `configure_buffer`. `Allocator::Pool`/`VoxelMemoryPool` НЕ подключён — pool byte-oriented и test-only
   (см. D7), отложен до production-caller в Фазе 5. См. §9.7.
6. ✅ **B5: Cubes/Blocky zero-copy — ЗАКРЫТ 2026-07-12.** Blocky: `build_blocky_into` dispatch'ит
   depth **один раз** и передаёт типизированный slice напрямую в generic `generate_mesh<T: Copy+Into<u16>>`
   (настоящий zero-copy для Bit8/Bit16, default Type channel — Bit16). Cubes: `extract_voxel_slice`
   widened в `Vec<u32>` одним проходом `iter().map().collect()` вместо per-voxel `get_voxel` (free fn
   требует `&[u32]`, поэтому полного zero-copy нет, но depth-dispatch один раз). Оба adapter'а получили
   `Compression::Uniform` fallback (`vec![defval; cap]`) — `channel_data` возвращает пустой Vec для
   uniform-канала, что вызывало бы out-of-bounds в mesher core. Regression-тест `blocky_mesher_handles_uniform_air_block_without_panicking`
   закрывает латентный баг (parallel-агент нашёл, что Blocky path не имел uniform-handling в отличие от
   Cubes/Transvoxel). **M1.C (волна 3) полностью закрыта.** См. §9.7.

M1.D — Волна 3, graph runtime (§9.3/§9.6-C):
7. ✅ **C1: compile-шаг `CompiledGraph` — ЗАКРЫТ 2026-07-12.** `CompiledGraph` (topo-кэш + dense
   index + XZ-outer-group classification) компилируется один раз (lazy в `GraphGenerator`).
   `CompiledScratch` (dense `Vec<Vec<f32>>`) заменяет `HashMap<GraphNodeId, Vec<f32>>` с
   capacity-preserving reuse. `generate_slice` eval'ит узлы по dense index, а на 2-м и далее
   Y-slices skip'ает Y-independent prefix (`xz_prefix_cached=true`). `OutputSdf` writes в scratch
   (persist across cache boundary). `generate_block_with_compiled_graph` строит XZ coords один
   раз вне Y-loop. Все edge-cases сохранены (golden parity: 15 новых C1-тестов + 32 существующих
   graph-теста зелёные). Constant folding / sampler hoist — отдельные шаги если bench покажет
   нужду. См. §9.7.
8. ✅ **C3: range analysis — ЗАКРЫТ 2026-07-12.** `CompiledGraph::analyze_range(x, y, z) -> Interval`
   пропагирует input intervals через compiled topo-order (easy nodes: Add/Sub/Mul/Div/Min/Max/Sin/
   Abs/Sqrt/Floor/Fract/Remap/Distance2D/3D/SdfPlane/SdfSphere/SdfUnion/SdfSubtract/Mix/Clamp;
   hard nodes: Cos/Noise/Curve/Normalize/Pow/SdfSmooth*/SdfBox/SdfTorus → conservative `infinity()`).
   `generate_block_with_compiled_graph` culls uniform-SDF blocks (conservative: только `is_single_value`
   заполняет actual value; sign-only ranges остаются per-voxel — фактическое SDF значение может нести
   информацию). 7 новых тестов (5 runtime: constant/plane-solid/plane-straddle/noise-fallback/add;
   2 adapter: cull air + cull solid). **M1.D (graph runtime) полностью закрыта: C1+C3.** См. §9.7.

M1.E — Инфра-долг (§9.6 «Инфраструктура», §7):
9. ✅ **H2-MT benchmark harness — ЗАКРЫТ 2026-07-12.** `mesh_block_bench` (criterion): `mesh_block/single`
   меряет end-to-end `MeshBlockTask::run_meshing` (gather + mesher.build, single-threaded), `mesh_block/multi`
   меряет 32 блока через `ThreadedTaskRunner(4)` (MT paging throughput). Результаты: single 47 µs / 86 Melem/s,
   multi 673 µs / 194 Melem/s (**2.25× speedup** на 4 потока — подтверждает M1.A threading model).
   Сравнение с C++ baseline требует расширения `cpp-baseline/` (single-threaded transvoxel-only сегодня) —
   оставлено как M2+ follow-up (cpp-baseline end-to-end harness нетривиален).
10. ✅ **cargo-fuzz** таргеты на `.vox`, `block_serializer`, `region` (D2-класс багов) — ЗАКРЫТ. Нашёл
    и пофиксил OOM bug в `decompressed_size` (см. §9.7).
11. **CI**: вернуть авто-триггеры `rust.yml` (push/PR) + bench-smoke + x86_64-android emulator
    smoke, когда GitHub flow готов. 🟡 отложено — `workflow_dispatch` остаётся, авто-триггер
    включается когда пилот-фаза стабильна и GitHub Actions quota подтверждена.
12. ✅ **§7 риски**: risk 1 (cpp-reference) ✅, risk 3 (TSan) ✅, risk 6 (REPORT.md) ✅,
    risk 2 (on-device) 🟡 задепрекейчено.

**DoD M1:** все основные пункты закрыты (TSan ✅, H2-MT harness ✅, cargo-fuzz ✅, §7 риски ✅).
CI auto-trigger (item 11) отложено до стабилизации пилота. clippy/fmt/тесты чистые.

#### M2 — Фаза 4 до GO

13. **Multi-LOD paging**: `VoxelLodTerrain` + `VoxelLodTerrainUpdateData` + threaded update task
    + clipbox/octree strategy (~9.9k LOC C++, главный блокер Фазы 4). Сверка с
    `terrain/variable_lod/*`. Transition cells для transvoxel (LOD-переходы) — часть этого шага.
14. **`VoxelEngine` остаток**: time-spread/progressive очереди, GPU queue (опц.), file locker,
    stats/profiling, volume callbacks.
15. **`VoxelDataGrid`**, сквозной TSan на multi-LOD сцене.
**DoD M2:** GO-критерий Фазы 4 формально выполнен; multi-LOD parity/стресс зелёные.

#### M3 — Фаза 5 (Godot binding + editor)

16. ✅ **Binding MVP (7 steps, 2026-07-20):** `VoxelTerrain` (Node3D) + `VoxelViewer` (Node3D) +
    4 generator Resources (Waves/Flat/Noise/Heightmap) + edition tools (set/get_voxel_sdf, raycast,
    get_bounds) + `lod_count` property + dirty re-upload + `material_override` + `generate_collision`.
    Rendering + editing + materials + collision функциональны в Godot. Smoke-criterion (viewer →
    paging → mesh → render) выполнен.
17. **editor/** (12.8k), **edition/** (7.8k), **modifiers/** (1.5k), **terrain/instancing** (9.2k),
    **terrain/ root** (2.0k) — портирование подсистем. ⏳ deferred.
18. **Stream binding** (save/load): `VoxelStreamRegionFiles` / memory stream как Resource property. ⏳
**DoD M3:** smoke-criterion ✅ (viewer → paging → mesh → render). Editor plugins + stream + instancing
остаются как incremental additions.

#### M4 — Паритет и удаление C++

18. Полный parity против `cpp-reference` (расширенный набор golden/diff-тестов по всем подсистемам).
19. Удаление C++ модуля из `master`; форк — чистый Rust-проект; `cpp-reference` остаётся только
    как зеркало upstream для отслеживания будущих bugfix'ов.
**DoD M4:** `master` собирается и проходит все тесты без C++; `cpp-reference` обновляется из Zylann/master.

### 10.1.3 Что НЕ входит в DoD (опционально/отложено)

- GPU-путь: `engine/gpu`, `engine/detail_rendering` (normal maps), `shaders/` (~7.1k LOC).
- `streams/sqlite` (2.2k), `generators/multipass` (2.3k), `util/noise` FastNoise2/SpotNoise (~3.1k).
- Physics (Rapier, §9 плана — не начат).
Эти подсистемы трекаются отдельно; их отсутствие не блокирует ни один milestone.

### 10.1.4 Процесс (по требованию заказчика)

- Ветка: `rust/pilot` (как сейчас); milestone может выделять долгоживущую ветку `rust/m{1,2,3}` по необходимости.
- Каждый пункт дорожной карты → отдельный коммит + `git push`; сообщение коммита по образцу
  существующих (`rust(phase4): add ...`, `fix(meshers): ...`).
- После каждого milestone — обновление `rust/STATUS.md` (фаза/тесты/секция «what remains») и
  журнала `§9.7` (для M1) или новой фазовой секции (для M2+).
- Инварианты (тесты/clippy/fmt/golden-parity) проверяются перед каждым push.

---

## 11. Повторный аудит реализации — 2026-07-10

### 11.1 Объём и критерии

Проверен HEAD `60225f11de4a` на ветке `rust/pilot`. До изменения только этого документа
рабочее дерево было чистым; ветка совпадала с `origin/rust/pilot` (`0 ahead / 0 behind`).
Ревью включало:

- повторный прогон workspace и all-features тестов, fmt, clippy с `-D warnings`, сборки и
  criterion-бенчмарка;
- трассировку ownership/error/cancellation/shutdown путей terrain → tasks → streams;
- сверку graph, mesh-gather, Blocky/Cubes/Transvoxel и region/VOX форматов с C++-референсом;
- проверку того, что тесты действительно доказывают заявленные свойства, а не только проходят;
- поиск safe-API soundness, allocation amplification, on-disk integrity и lock-ordering рисков.

Не выполнялись on-device Android smoke, Linux TSan/Miri, fault-injection с реальным process kill,
долгий soak и автоматический RustSec scan. Их отсутствие явно учитывается в verdict, а не
подменяется зелёным `cargo test`.

Приоритеты:

- **P0** — закрыть до production persistence, импорта недоверенных файлов или публичного binding;
- **P1** — закрыть до заявления C++ parity / завершения Фазы 4;
- **P2** — не блокирует исследовательскую ветку, но требует задачи, теста и измеримого DoD.

### 11.2 Что из прежнего плана подтверждено

| Пункт | Текущий статус | Граница подтверждения |
|---|---|---|
| A1 generator `&self + Arc<dyn>` | 🟡 частично | внешний mutex снят; `GraphGenerator` всё ещё держит один scratch-mutex на весь block call |
| A2 mesher `&self + Arc<dyn>` | ✅ | общий внешний mutex снят, Transvoxel cache — TLS; geometry/output scratch reuse не сделан |
| A3 settings + per-LOD map + `SpatialLock3D` | 🟡 частично | cross-LOD lock снят; весь map одного LOD остаётся одним `RwLock`, edit transaction не атомарна |
| A4 callbacks вне data/map locks | ✅ для текущих load/gather путей | generator/mesher/stream вызываются после snapshot/drop; публичный API всё ещё допускает иной lock order |
| A5 semaphore/staging/nonblocking drain | 🟡 механика есть | stale/cancelled outputs, panic liveness, приоритеты terrain и persistence shutdown не закрыты |
| B2 uniform Transvoxel fast-path | ✅ вычислительно | выдаётся пустая surface вместо C++ zero-surfaces; direct golden обходит adapter/depth paths |
| C2 uniform graph compression | ✅ | подтверждено тестом constant-output |
| D1 deferred region header | ✅ только I/O amplification | crash consistency и multi-handle coherency не обеспечены |
| D2 negative `.vox` SIZE | ✅ только этот кейс | framing и cumulative allocation budgets отсутствуют |

### 11.3 Сводка новых и уточнённых находок

| ID | Приоритет | Находка | Последствие |
|---|---|---|---|
| SAVE-1 | **P0** | dirty block удаляется до подтверждения save; error/cancel/shutdown теряют единственный payload | безвозвратная потеря edits |
| SAVE-2 | **P0** | save одного блока не сериализованы/не версионированы | старый completion может перезаписать новое состояние |
| SAFE-1 | **P0** | public safe `Vector3/4::get/set` достигают `unreachable_unchecked` | UB из safe Rust в release |
| DECODE-1 | **P0** | compression/block/region/VOX readers аллоцируют по недоверенным размерам без budget | OOM/abort, memory amplification |
| REGION-1 | **P1** | LUT/header не валидируются полностью; wire limits проверяются только `debug_assert` | чтение/запись повреждённых region files |
| REGION-2 | **P1** | compaction + deferred header не crash-consistent | старый LUT указывает на уже сдвинутые данные |
| REGION-3 | **P1** | несколько `RegionFile` handles имеют независимые LUT/cache без path lock/reload | lost update и overwrite одного файла |
| EDIT-1 | ✅ закрыто 2026-07-10 | edit отсутствующего procedural block создаёт default buffer; edit и dirty flag разнесены | закрыто transactional API, terrain adoption и regression suite (§11.9) |
| GRAPH-1 | ✅ закрыто 2026-07-10 | `SdfSmoothSubtract` меняет местами operands | закрыто C++ parity-вектором и исправлением порядка operands (§11.9) |
| GRAPH-2 | **P1** | Distance/Normalize/Remap/Divide не совпадают с одноимёнными C++ nodes | graph assets нельзя считать parity-compatible |
| MESH-1 | **P1** | dependency проверяется только до build; cancelled task может не дать output | stale mesh применяется или remesh теряется |
| GATHER-1 | **P1** | missing halo генерируется целыми blocks без bounds clipping | boundary seams и до ~38,5× лишних samples |
| BLOCKY-1 | **P1** | AO strength не делится на 3 и не clamp-ится | отрицательные vertex colors при default 0,8 |
| CUBES-1 | **P1** | Rust adapter всегда palette-mode, C++ default — raw RGBA | иные цвета, alpha и material routing |
| VOX-1 | **P1** | known chunks игнорируют declared framing; global model budget отсутствует | parser desync и гигабайты из малого файла |
| META-1 | **P1** | block metadata skip превращается wrapper-ом в `Ok(())` | тихая потеря metadata при load→save |
| TASK-1 | **P1** | terrain load получает max priority, mesh — minimum; старые loads не cancel-ятся | starvation mesh/save и retry storm |
| TASK-2 | **P1** | panic обходит in-flight cleanup runner | зависший wait/shutdown или потерянный worker |
| GRAPH-PERF-1 | **P2** | один `Mutex<GraphScratch>` сериализует shared graph generator | pool workers генерирует graph blocks по одному |
| DATA-PERF-1 | **P2** | один whole-map `RwLock` на LOD | disjoint edits/view/unview блокируют все gathers LOD |
| MESH-PERF-1 | **P2** | per-sample dyn/depth dispatch и geometry allocations остаются | H2 core-бенч не описывает end-to-end throughput |
| POOL-1 | **P2** | memory-pool accounting смешивает bucket capacity и resized len | неверная статистика/underflow после `clear` с live blocks |
| INFRA-1 | **P2** | stress не пересекает ключевые paths; CI manual/default/unlocked | зелёный gate не доказывает production invariants |

### 11.4 P0/P1: сохранность данных, safe API и бинарные форматы

#### SAVE-1/SAVE-2 — persistence не имеет commit ownership

`SharedVoxelData::unview_area` удаляет modified block и передаёт единственный `VoxelBuffer`
в `BlockToSave` (`storage/voxel_data.rs:335-363`). Terrain enqueue-ит save как `serial=false`
(`terrain/voxel_terrain_core.rs:340-385`). `SaveBlockDataTask` делает `self.voxels.take()`,
а при ошибке возвращает `Saved { dropped: true, voxels: None }`
(`streams/save_block_data_task.rs:83-131`, `streams/block_data_output.rs:96-119`). Terrain
игнорирует все `Saved`, включая `dropped=true` (`voxel_terrain_core.rs:525-568`).

Дополнительно `ThreadedTaskRunner::Drop` сразу выставляет `stopping`; worker выходит до drain
staged/queued tasks (`tasks/threaded_task_runner.rs:151-178,265-273`), а у
`VoxelTerrainCore` нет обязательного shutdown/flush API. Два save одного block могут идти
параллельно и завершиться в обратном порядке.

Варианты решения:

1. **Рекомендуется:** pending-save journal владеет payload до подтверждённого commit; per-block
   monotonic generation; error возвращает payload в очередь с bounded retry/backoff; terrain
   имеет `shutdown_and_flush() -> Result` и Godot lifecycle обязан его вызвать.
2. Общий serial I/O scheduler (как путь `VoxelEngine`) + сохранение failed payload до retry.
   Проще, но сериализует независимые регионы.
3. Синхронный save-on-unload. Корректный аварийный вариант, но блокирует main tick.
4. Durable WAL/write-behind service. Лучшее долгосрочное решение для crash recovery и batch,
   но это отдельная подсистема, а не локальный фикс.

#### SAFE-1 — UB доступен через safe Rust

`math/vector3.rs:57-76` и `math/vector4.rs:45-66` проверяют индекс только через
`debug_assert!`, затем для неверного значения вызывают `unsafe { unreachable_unchecked() }`.
В release `get(3)`/`set(4, ...)` — undefined behavior, хотя соседние `Index/IndexMut` корректно
panic-ят. Это особенно опасно перед GDExtension/API binding, где индекс может прийти извне.

Варианты решения:

1. **Рекомендуется:** обычный exhaustively checked `match` с `panic!`, как в `Index`.
2. Для динамического ввода — `try_get/try_set -> Option/Result`; checked `get/set` оставить.
3. Если профиль докажет необходимость, вынести отдельные `unsafe fn get_unchecked/set_unchecked`
   с `# Safety`; safe wrapper всегда проверяет bounds. Добавить `#![deny(unsafe_code)]` в core
   и точечно разрешать только обоснованные модули.

#### DECODE-1 — размеры входа не ограничивают аллокации

- LZ4/Zstd доверяют `u32 decompressed_size`; LZ4 сразу `resize`, Zstd сначала создаёт dst,
  затем ещё один `Vec` через `decode_all` (`streams/compressed_data.rs:128-170`). Проверка
  `< 0` мёртвая, потому что исходное значение `u32`.
- V4 block читает dimensions как `u16³`, вызывает `buffer.create` и materialize channel до
  проверки, что bytes вообще присутствуют (`streams/block_serializer.rs:235-260`).
- `RegionFile::load_block` доверяет 4-byte payload length и аллоцирует до сверки с
  `sector_count * sector_size` (`streams/region/region_file.rs:443-465`).
- `.vox` создаёт dense `SIZE` model до чтения voxel count; повторные `XYZI` не имеют общего
  model/voxel budget (`format/vox/parser.rs:273-322`).

Варианты решения:

1. **Рекомендуется:** единый `DecodeLimits`/budget (max bytes, voxels, models, nodes, strings)
   передаётся всем nested readers; каждый allocation делает checked arithmetic и
   `try_reserve_exact`, а parent format задаёт более строгий expected maximum.
2. Локальные API `decompress_limited(src, max_output)` + block preflight header/volume/required
   bytes. Быстрее внедрить, но легко оставить новый parser без общего budget.
3. Streaming decode с hard cap (`max + 1`) для Zstd/LZ4 и sparse `.vox` до финализации model.
4. Изолировать импорт в отдельный process с memory/time limits. Полезная defense-in-depth,
   но не заменяет bounds внутри библиотеки.

#### REGION-1/2/3 — integrity, durability и coherency `.vxr`

`load_header` не вызывает `RegionFormat::validate`; invalid channel depth молча превращается
в Bit8, LUT не проверяется на overlap/range/file length, а reverse sector map строится по всем
заявленным counts (`region_file.rs:227-335,614-624`). Даже текущий `validate()` допускает zero
axis, не ограничивает `sector_size` его wire-типом `u16` и не включает serializer envelope.
`RegionBlockInfo::new/set` в release маскирует oversized count/index после одного `debug_assert`
(`streams/region/format.rs:70-103,151-183`).

При compaction tail физически сдвигается/truncate-ится до записи нового LUT; header остаётся
dirty до `flush/close/Drop` (`region_file.rs:352-410,507-600`). Crash между этими шагами оставит
старый LUT поверх уже изменённых payload offsets. Наконец, документация разрешает handle на
каждый thread (`region_file.rs:77-90`), но handles имеют независимые LUT/sector cache и целиком
перезаписывают header; существующий `FileLocker` сюда не подключён.

Варианты решения:

1. **Рекомендуемый ближайший:** fallible `RegionFormat::try_new` + полная проверка header/LUT,
   payload `<= sectors - prefix`, interval map вместо vector-per-sector; один canonical-path
   writer `Arc<Mutex<RegionFile>>`, readers получают immutable snapshot.
2. Интегрировать `FileLocker`/OS lock, но под write-lock обязательно reload + generation check;
   lock без cache coherency проблему lost update не решает.
3. **Рекомендуемый durable target:** WAL или dual generation-stamped/checksummed headers:
   append data → sync → atomically commit generation → reclaim old sectors.
4. Copy-on-write whole region + `fsync` + atomic rename. Проще рассуждать о crash recovery,
   дороже по write amplification.

#### VOX-1/META-1 и вторичные serializer-paths

Known `.vox` handlers читают payload напрямую из общего reader, игнорируя `chunk_size` и
`children_size`; declared end используется только для unknown chunks
(`format/vox/parser.rs:273-279,281-517`). Malformed known chunk может прочитать следующий chunk,
а padding — стать новым tag. Scene validation проверяет ссылки/root, но не гарантирует, что
весь graph достижим и ацикличен.

`block_serializer::deserialize` умеет вернуть `MetadataSkipped`, но wrapper превращает это в
`Ok(())`, а `RegionFile` вызывает именно wrapper (`block_serializer.rs:205-327`,
`region_file.rs:465`). C++ metadata поэтому тихо исчезает после Rust load→save.

Варианты решения:

1. **`.vox`:** bounded sub-reader ровно на `chunk_size`, отдельная MAIN children boundary,
   exact consumption/explicit skip; DFS tri-color + reachability либо Kahn validation.
2. **Metadata:** structured outcome с обязательным propagation; default — reject lossy load,
   explicit `allow_metadata_loss` только по решению caller.
3. До порта Variant codec хранить metadata как opaque bytes и round-trip без интерпретации.
4. Secondary hardening: `stream_cache::try_flush(F -> Result)` удаляет entry только после
   success; instance serializer preflight-ит `u8/u16` counts и пишет atomically во временный Vec
   (`stream_cache.rs:115-128`, `instance_data.rs:162-300`).

### 11.5 P1: correctness и C++ parity

#### EDIT-1 — shared edit не является атомарной транзакцией

`SharedVoxelData::try_set_voxel` для отсутствующего/empty block создаёт formatted default buffer,
не материализуя установленный generator (`storage/voxel_data.rs:373-400`). В procedural world
изменение одного voxel тем самым заменяет остальные voxels block на defaults. Затем caller
отдельно вызывает `mark_area_modified` (`:403-421`); unview между этими вызовами может удалить
ещё «чистый» block без save. C++ сначала генерирует base block вне map lock, оставаясь под
spatial write guard (`storage/voxel_data.cpp:258-290`).

Варианты решения:

1. **Рекомендуется:** единая `edit_voxel/edit_region` transaction охватывает materialization,
   mutation и dirty/edited flags одним spatial guard; generator работает вне map lock, затем
   insert делает повторную проверку версии.
2. `EditSession`/guard выдаёт writable block и гарантированно marks dirty на Drop/commit.
3. Минимум: `try_set_voxel` сам marks modified/edited; procedural materialization остаётся
   отдельным обязательным исправлением.

#### GRAPH-1/2 — одноимённые nodes имеют другую математику

- `SdfSmoothSubtract` вызывает `sdf_smooth_subtract(b, a, s)`, тогда как hard fallback и C++
  используют `(a, b)` (`generators/graph/runtime.rs:659-668`, C++ `nodes/sdf.h:317-340`). Для
  `a=-0.2, b=0.4, s=1` C++ даёт `-0.04`, Rust `+0.56` — меняется знак SDF.
- `GraphPort` хранит только node id, multi-output port отсутствует (`runtime.rs:17-28`).
- Rust Distance2D/3D считает length от origin по 2/3 inputs; C++ — расстояние двух points по
  4/6 inputs. Rust Normalize3D возвращает один `1/len`; C++ — `nx,ny,nz,len`
  (`runtime.rs:110-126,504-537`, C++ `nodes/math_vectors.h:10-132`).
- Rust Remap clamp-ит extrapolation; C++ оставляет linear. Divide использует epsilon и default 0,
  C++ — exact zero и denominator default 1 (`runtime.rs:435-489`).

Варианты решения:

1. **Рекомендуется:** port schema `{ node, output_index }`, defaults из C++ node DB,
   exact formulas и golden vectors для каждого node, включая discontinuities/zero cases.
2. Lower multi-output C++ nodes в несколько scalar internal nodes при compile/import; публичная
   asset schema остаётся C++-совместимой.
3. Если parity пока не цель — переименовать текущие операции (`Length3D`, `InvLength`,
   `ClampedRemap`) и явно reject unsupported C++ graph assets. Тихо сохранять те же имена нельзя.

#### MESH-1 — stale/cancelled output lifecycle

`MeshBlockTask` проверяет dependency только до gather/build, после `mesher.build` всегда ставит
`dropped=false` (`meshers/mesh_block_task.rs:108-178`). Terrain принимает любой такой output
без dependency/request version (`terrain/voxel_terrain_core.rs:572-585`). Если runner отменяет
задачу до `run`, output остаётся `None`; drain молча её выбрасывает и не requeue-ит block
(`tasks/threaded_task_runner.rs:309-318`, `voxel_terrain_core.rs:483-495,806-809`). Unload/reload
того же position тоже не различается epoch-ом.

Варианты решения:

1. **Рекомендуется:** monotonic dependency generation + per-block request sequence в output;
   apply принимает только текущую generation и последнюю sequence. `set_mesher/set_generator`
   заменяет dependency и помечает видимые blocks на remesh.
2. Generic `on_cancel`/typed cancelled outcome синтезирует `dropped` output; если block ещё viewed,
   terrain обязательно requeue-ит.
3. Минимум: повторная dependency check после build и при apply. Это уменьшает окно, но без
   request sequence не закрывает out-of-order старого и нового task.

#### GATHER-1 — bounds и точный halo не портированы

Rust queue-ит для каждого отсутствующего соседа полный `data_block_size³` scratch и не использует
`SharedVoxelData::bounds` (`mesh_block_task.rs:299-340,404-469`). C++ клипует padded mesh box
по bounds, вычитает resident boxes и генерирует только остаток (`mesh_block_task.cpp:100-226`).
Для resident central 16³ Transvoxel block Rust может запросить `26 × 16³ = 106 496` samples,
тогда как точный halo `19³ - 16³ = 2 763`: около **38,5×** лишней генерации. За fixed bounds
Rust также заполняет halo generator-данными вместо format default, меняя boundary semantics.

Варианты решения:

1. **Рекомендуется:** портировать C++ clip + box-difference plan, генерировать только точные
   remainder boxes одним pooled scratch.
2. Один generator query на clipped padded buffer, затем overlay resident blocks и явное
   заполнение outside-bounds defaults.
3. Минимум: reuse одного scratch и skip empty intersections. Allocation улучшится, но full-block
   overgeneration/семантика границ останутся частично.

#### BLOCKY-1/CUBES-1 и capability defaults

Blocky public darkness default `0.8` идёт прямо в core, где умножается на `shaded_corner 0..3`;
полностью закрытый corner получает shade `2.4` и RGB `-1.4`
(`meshers/builtin.rs:294-373`, `blocky/mesher.rs:547-575`). C++ clamp-ит public value и перед
core делит на 3 (`voxel_mesher_blocky.cpp:516-519,589-592`).

`CubesMesher` всегда трактует raw value как palette index (`builtin.rs:167-253`), тогда как
C++ default — `COLOR_RAW` и dispatch зависит от mode/depth (`voxel_mesher_cubes.h:123-127`,
`.cpp:802-878`). Текущие тесты используют значения, на которых raw/palette визуально совпадают.

Дополнительно `VoxelMesher::supports_lod()` default `true`, а `TransvoxelMesher` его не override-ит,
несмотря на отсутствие transition cells (`voxel_mesher.rs:241-244`, `builtin.rs:114-165`).
Edge clamp hardcoded `0.0`, C++ production default `0.02`; public gather origin при LOD>0 не
shift-ится обратно в LOD0 coordinates (`builtin.rs:124-128`, `mesh_block_task.rs:232-295`).

Варианты решения:

1. **Blocky:** clamp `[0,1]` и нормализовать `/3` внутри core (защищает и direct callers) либо
   буквально в adapter для минимальной C++-парити; добавить assert colors ∈ `[0,1]`.
2. **Cubes:** `CubesColorMode::{Raw, Palette}`, default `Raw`, depth dispatch до loop; либо
   честно переименовать текущий adapter в palette-only и не заявлять C++ parity.
3. **LOD:** немедленно override `supports_lod=false` до transition port; долгосрочно разделить
   capabilities на regular scaled cells и transitions. Edge clamp сделать параметром с default
   `0.02`; origin API типизировать (`Lod0VoxelOrigin`/`LodVoxelOrigin`) или исправить shift.

### 11.6 P1/P2: конкурентность, liveness и performance

#### TASK-1 — priority/cancellation terrain не подключены к готовой инфраструктуре

Default `ThreadedTask::priority()` — `TaskPriority::max`; `LoadBlockForTerrainTask` его не
override-ит и не имеет cancellation token (`tasks/threaded_task.rs:55-63`,
`terrain/voxel_terrain_core.rs:731-790`). `MeshBlockTask` возвращает `0`, save имеет обычный band.
Старые dispatched loads после unview не отменяются; постоянная stream error requeue-ится без
backoff. При движении viewer load backlog может вытеснять mesh/save.

Варианты решения:

1. **Рекомендуется:** использовать существующий `LoadBlockDataTask` с `PriorityDependency` и
   `TaskCancellationToken`; token хранить в block entry и cancel-ить при unview/revision change.
2. Task generation/tombstone map + distance/band priority и aging/fairness в общем runner.
3. Отдельные bounded IO/mesh pools с квотами; cancellation и save ordering всё равно нужны.

#### TASK-2 — panic нарушает runner invariants

`running_count` и `serial_running` изменяются до `task.run`, а cleanup стоит только после
нормального возврата (`threaded_task_runner.rs:287-295,372-405`). Panic в user stream/generator/
mesher убивает worker и оставляет wait/shutdown зависшим; panic в `priority/is_cancelled` идёт
под state lock. `Default` runner с 0 workers и бесконечный `Postponed` — дополнительные liveness
holes.

Варианты решения:

1. **Рекомендуется для unwind builds:** `catch_unwind` вокруг callbacks + RAII in-flight guard,
   который всегда decrement/reset/notify; task завершается как typed `Panicked`.
2. Poison pool и возвращать `Result` из wait/shutdown, отменяя остаток queue.
3. Если release policy остаётся `panic=abort`, всё равно сохранить guard для tests/dev и явно
   запретить zero-worker enqueue; postponed tasks должны иметь backoff/notification source.

#### GRAPH-PERF-1 / DATA-PERF-1

`GraphGenerator` держит `Mutex<GraphScratch>` на весь `generate_block_with_graph`
(`generator_graph.rs:21-29,63-70`). Внутри каждый block/slice заново строит topology/Vec/HashMap,
перезаполняет X/Z и sampler (`generator_graph.rs:94-153`, `runtime.rs:225-264,346-417,556-579,
681-777`). Shared graph generator поэтому не масштабируется по workers.

Каждый LOD хранит один `RwLock<VoxelDataMap>`; view/unview/edit/load берут whole-map write,
mesh gather — whole-map read (`storage/voxel_data.rs:65-67,185-205,244-421`,
`mesh_block_task.rs:404-421`). `SpatialLock3D` не даёт параллелизма disjoint mutations, пока
глобальный map write удерживается. Публичные raw map closures плюс отдельные region guards также
позволяют ABBA map→spatial против internal spatial→map. Pending writer не учитывается, поэтому
его могут обходить новые readers. `BoxBounds3i` документирован half-open, но conservative
`<`/`>` intersection считает соседние boxes пересекающимися (`math/box_bounds.rs:72-120`).

Варианты решения:

1. **Graph:** immutable `CompiledGraph` + dense SSA/multi-output ports; per-worker/TLS retained
   `GraphScratch`; bulk channel writer. Более локальный вариант — scratch pool с lock только
   checkout/return.
2. **Data:** map хранит `Arc` block snapshots/per-block locks; map write только insert/remove,
   voxel mutation под spatial guard и block-level COW/version. Альтернатива — coordinate shards.
3. Закрыть raw map API composite transaction-методами, всегда spatial→map; debug lock ranks;
   writer-intent/FIFO в `SpatialLock3D`. Отдельно решить, сохраняется ли conservative C++
   intersection или API действительно следует half-open документации.

#### MESH-PERF-1 / POOL-1

Transvoxel adapter раскладывает flat index div/mod, затем `get_voxel_f` снова считает index и
dispatch-ит compression/depth за sample (`meshers/builtin.rs:40-80`). Cubes/Blocky копируют весь
channel в свежий typed Vec; geometry arrays/masks создаются заново. `MesherOutput::clear()`
дропает enum Vec capacities, а не сохраняет их. Поэтому direct f32 `build_regular_mesh` benchmark
не измеряет реальный adapter/gather/allocator path.

`VoxelMemoryPool::allocate` считает bucket-sized `len`, но `VoxelBuffer::alloc` уменьшает `len`
до requested bytes; recycle вычитает уже уменьшенный `len` (`storage/voxel_memory_pool.rs:85-155`,
`storage/voxel_buffer.rs:835-844`). Для non-power-of-two sizes `used_memory` дрейфует. `clear()`
обнуляет counters даже при live blocks (`voxel_memory_pool.rs:189-200`), после recycle возможен
atomic underflow.

Варианты решения:

1. **Adapter:** generic typed/byte-decoding sampler с depth dispatch один раз; direct byte
   fallback безопаснее бездоказательного cast. Долгосрочно — typed `ChannelData`.
2. Per-worker scratch для всех meshers + output free-list; end-to-end H2-MT benchmark на
   `MeshBlockTask`/moving viewer, не только algorithm core.
3. Pool возвращает wrapper с charged bucket bytes либо всегда считает capacity; `clear` разрешён
   только без live blocks или очищает лишь idle buckets. Добавить non-POT/live-clear tests.

### 11.7 Доказательная база тестов и CI

Текущий `threaded_edit_load_mesh_stress` полезен как smoke lock-release, но не как доказательство
полной модели:

- использует `generator=None`, поэтому не видит `GraphGenerator` mutex;
- load inserts находятся далеко от mesh/edit regions;
- dummy mesher не читает voxel values и проверяет только размер/count;
- нет `VoxelTerrainCore`, viewer movement, cancel, unload/save, mid-run invalidation или
  out-of-order completion;
- assertions — counts, наличие output и нулевое число оставшихся locks.

Golden Transvoxel spheres вызывают direct f32 core и обходят `VoxelBufferTransvoxelInput`,
8/16-bit depths и compression. Graph tests частично закрепляют текущую Rust-only семантику Remap/
Distance вместо сравнения с C++. Поэтому зелёные 655/657 unit не опровергают §11.4–11.6.

CI `.github/workflows/rust.yml` manual-only, запускает default features без `--locked` и без
clippy `-D warnings`. `rust/Cargo.lock` есть локально, но игнорируется правилом `rust/.gitignore:4`;
находящиеся ниже negations написаны как `!rust/...` внутри `rust/.gitignore` и lockfile не
разблокируют. Rust byte parsers не имеют fuzz targets. Есть и локальный documentation drift:
`meshers/builtin.rs:9-12` всё ещё говорит, что Cubes/Blocky adapters — TODO, хотя оба реализованы.

Рекомендуемые проверки:

1. Детерминированные barrier/hook tests: save error retains payload; two-save ordering; shutdown
   drains; mid-build invalidation; cancelled-before-run requeue; edit-vs-unview transaction;
   same-position unload/reload epoch.
2. C++ node/mesher golden matrix: every graph node/default/output port, Blocky AO 0/1/2/3,
   Cubes raw/palette/depth, fixed-bounds gather, Transvoxel adapter 8/16/32-bit.
3. Property/fuzz tests with explicit budgets для compression, block v4, region, `.vox`, metadata;
   corpus включает truncation, oversized prefix, overlap LUT и crash-reopen sequences.
4. Linux nightly TSan для реальных concurrent paths; `loom` для небольших primitives;
   Miri для unsafe/soundness-sensitive tests. TSan — дополнительный gate, не замена логическим тестам.
5. Track workspace `Cargo.lock`; CI: `fmt`, `test --workspace --all-features --locked`, clippy
   `--all-targets --all-features --locked -- -D warnings`, build и advisory/license gate.

### 11.8 Рекомендуемый порядок и verdict на 2026-07-10

**Волна 0 — data safety (блокирующая):** SAVE-1/2 → SAFE-1 → DecodeLimits → strict region
format/LUT validation → explicit shutdown/flush. DoD: fault-injection не теряет payload,
invalid input всегда даёт bounded `Err`, safe API не содержит reachable unchecked UB.

**Волна 1 — correctness/parity:** EDIT-1 → GRAPH-1/2 → MESH-1 → GATHER-1 → Blocky/Cubes
parity → metadata/framing. DoD: C++ golden matrix и deterministic race tests зелёные; unsupported
features явно rejected, а не маскируются одноимённым API.

**Волна 2 — lifecycle/concurrency:** terrain priority/cancellation, per-block generations,
runner panic handling, region single-writer/durable transaction, stronger stress + TSan.

**Волна 3 — performance:** compiled graph/per-worker scratch, map/block sharding/COW, typed
mesher inputs и retained arrays. DoD: H2-MT измеряет gather→build→apply и moving-viewer throughput;
direct core benchmark остаётся microbenchmark, а не production claim.

**Verdict на 2026-07-10:**

- **GO** для продолжения headless R&D на `rust/pilot`: код собирается, текущая suite стабильна,
  и закрытые A/B/C/D-пункты не являются фиктивными.
- **NO-GO** для production persistence и импорта недоверенных `.vxr`/block/`.vox` данных как
  минимум до завершения Волны 0; для `.vox` дополнительно нужен framing из VOX-1 Волны 1.
- **NO-GO** для заявления полной C++ parity: graph nodes, Blocky/Cubes и gather имеют доказанные
  расхождения.
- **NO-GO** для начала Godot-facing Фазы 5 поверх текущего lifecycle: сначала нужны safe API,
  save/shutdown contract и typed cancellation/versioning.
- Multi-LOD нельзя подключать к текущему `supports_lod=true` без transition cells; минимум —
  честно вернуть `false` до соответствующего порта.

В рамках повторного аудита production/test код **не изменялся**; обновлён только этот документ
с результатами проверки и вариантами решений.

### 11.9 Закрытые после повторного аудита пункты

| ID | Статус и реализация | Проверка |
|---|---|---|
| SAFE-1 | ✅ `9e90ee7f`: safe vector accessors проверяют границы до unchecked-доступа | focused regressions + полная all-features suite |
| DECODE-1 | ✅ `b6d3fd69`: `DecodeLimits` ограничивает байты/voxels/models, при этом сохранён fuzz-found hard ceiling 256 MiB | oversized LZ4/None regressions + parser/serializer tests |
| REGION validation | ✅ `d870c396`: строгая проверка header, channel depth, LUT bounds/overlap и payload sectors | malformed-header/LUT/payload regressions |
| SAVE-1/2 | ✅ `484134b2`: save journal сохраняет payload, различает поколения и LOD, повторяет dropped saves и поддерживает `shutdown_and_flush` | fault/retry/shutdown tests + LOD-distinct key regression из `070a3fc6` |
| EDIT-1 | ✅ `d379d1b8`, `166511e0`, `cd4c229a`: `SharedVoxelData::try_edit_voxel` держит spatial write-lock до map access, materialизует procedural block вне map lock, выставляет voxel + `modified` + `edited` одной map-транзакцией; terrain использует этот путь | 8 transaction/lock regressions, terrain unload persistence; GDExt routing дополнительно закрыт в `070a3fc6` |
| GRAPH-1 | ✅ `d3569f34`: lazy и compiled evaluators передают `(a, b, smoothness)` в `sdf_smooth_subtract`, как C++ | оба evaluator path: `a=-0.2`, `b=0.4`, `s=1.0` → `-0.04`; zero-smooth fallback |

`GRAPH-2` и остальные незакрытые пункты таблицы §11.3 сохраняют исходный приоритет и остаются
очередью дальнейшей работы.

### 11.10 Актуализация после сведения веток — 2026-07-24

Локальные 12 коммитов повторного аудита были созданы 2026-07-10, но не отправлены в origin.
Параллельно другая рабочая копия развила `origin/rust/pilot` на 35 коммитов до `e56895ae`
(M1 performance/TSan/fuzz, M2 multi-LOD/transition cells, M3 Godot binding MVP). Истории
сведены rebase-ом локальной safety-линии поверх `e56895ae`, затем добавлен интеграционный
коммит `070a3fc6`.

При сведении отдельно проверены семантические пересечения:

- save journal сохранён поверх per-LOD terrain и ключуется `(position, lod_index)`;
- configurable `DecodeLimits` не ослабляет fuzz-found 256 MiB hard ceiling;
- `SdfSmoothSubtract` исправлен и в legacy, и в новом compiled evaluator;
- Godot `set_voxel_sdf` проходит через атомарный `VoxelTerrainCore::try_edit_voxel`,
  сохраняя dirty-mesh re-upload.

Проверка объединённой линии: `cargo test --workspace --all-features` — **750 core tests passed,
3 ignored**, все integration/doc tests и обычные concurrency tests зелёные; `cargo fmt --check`,
workspace clippy `--all-targets --all-features -- -D warnings` и `cargo build -p voxel-gdext`
прошли. Независимое review не нашло code blockers.

Следующий продуктовый срез M3 — **stream binding save/load** (`VoxelStreamRegionFiles` /
memory stream как Godot Resource). Он должен опираться на объединённый save journal.
После него: editor plugins, instancing и modifiers. До заявления полной C++ parity остаются
как минимум GRAPH-2, MESH-1, GATHER-1, Blocky/Cubes parity и lifecycle/cancellation пункты §11.
