# План миграции godot_voxel → Rust GDExtension

> **Форк:** https://github.com/sandsaber/godot_voxel
> **Upstream:** https://github.com/Zylann/godot_voxel
> **Дата оценки:** 2026-07-03
> **Модель работы:** AI 24/7 + человек-архитектор

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

### Фаза 1: Чистое ядро (3-4 недели)
- `util/math`, `containers`, `string`, `io`, `memory` полностью
- `util/testing` (фреймворк для parity-тестов)
- **GO-критерий:** все unit-тесты проходят, perf ≥ C++

### Фаза 2: Мобильная валидация (2-3 недели) ⬅️ ПРИОРИТЕТ
- Cargo targets: `aarch64-linux-android`, `aarch64-apple-ios`, `x86_64-linux-android`
- Минимальный gdext "hello world", грузится в Godot 4 на Android-устройстве/эмуляторе
- CI pipeline: GitHub Actions с NDK
- **GO-критерий:** APK с Rust-gdext запускается на Android, класс виден в GDScript

### Фаза 3: Compute-слой (6-8 недель)
- `storage` (VoxelBuffer полный), `streams` (без SQLite)
- `meshers` (blocky, cubes — transvoxel уже есть)
- `generators` (noise — через FFI к FastNoise2, graph — позже)
- **GO-критерий:** генерация + meshing чанков работает на desktop + Android

### Фаза 4: Terrain + threading (8-10 недель) — самый сложный этап
- `util/tasks`, `util/thread` (свой thread pool, замена WorkerThreadPool)
- `terrain` (VoxelTerrain, VoxelLodTerrain)
- `generators/graph` (runtime, без редактора)
- **GO-критерий:** стриминг бесконечного terrain'а работает, нет race conditions
  (проверка под ThreadSanitizer/loom)

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
| 2026-07-03 | 0.7: parity framework + self-consistent golden | ✅ GoldenMesh JSON + comparator, sphere_16/32 |
| 2026-07-03 | 0.7 (real C++): table parity | ✅ Rust-таблицы byte-identical upstream C++ (`transvoxel_tables_parity`) |
| 2026-07-03 | 0.10: REPORT.md | ✅ Conditional GO; подробности в `REPORT.md` |
| — | 0.7 (mesh byte-parity vs C++) + 0.8 (C++ perf baseline) | ⏳ единственный открытый пункт — godot-cpp harness |

### Где остановились (для возобновления)

**Готово:** весь пилот, кроме mesh byte-parity vs C++ и C++ perf-baseline. 32/32 теста проходят,
clippy/fmt чист. voxel-core кросс-компилируется под все приоритетные мобильные/десктоп-таргеты.
Полный разбор и GO/NO-GO — в **`REPORT.md`**.

**Единственный открытый пункт (gating final Phase 0 sign-off):** godot-cpp mesh-harness.
Тело mesher'а зависит от Godot-типов (`util/godot` shim не имеет standalone-режима), поэтому
для mesh byte-parity нужен либо godot-cpp, либо Godot-source. План и оценка effort'а — в
`rust/cpp-baseline/README.md`. Тот же harness закроет и C++ perf-baseline (H2).

**Следующие шаги (по приоритету):**
1. **godot-cpp mesh harness** → закрыть H1 (regenerate golden из C++) и H2 (perf vs C++) одной задачей.
2. **Фаза 1** (math/containers/string/io/memory core) — не зависит от harness, можно начинать параллельно.
3. **Фаза 2 kick-off** — Android `.so` + минимальный gdext hello-world (`rust/scripts/android-build.sh` готов).

### Команды для возобновления работы
```bash
git clone https://github.com/sandsaber/godot_voxel.git
cd godot_voxel && git checkout rust/pilot
cd rust
cargo test                 # 32/32 должны пройти
cargo clippy --all-targets # должен быть чистый
cargo bench                # transvoxel benches (147–238 Melem/s)
./scripts/android-build.sh --so   # Android aarch64 .so (NDK r29 + rust-lld workaround)
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
- `block-mesh` — **кандидат для blocky mesher** (Фаза 3), зрелый.
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
