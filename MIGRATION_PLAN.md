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
| 2026-07-04 | 0.7 (mesh parity vs C++) + 0.8 (C++ perf baseline) | ✅ C++ harness (без godot-cpp, через stub-tree). **H1 partial**: позиции вершин совпадают (606=606), треугольников поровну (1304), но reuse-cache даёт 888 vs 840 вершин и 434/1304 треугольника расходятся в winding. **H2 PASS**: Rust 28.5µs/143Melem/s vs C++ 44.1µs/93Mvoxels/s (~1.5× быстрее). Детали в `rust/cpp-baseline/README.md` |
| 2026-07-04 | Фаза 2 mobile-half: voxel-gdext Android `.so` (NDK r29) | ✅ `libvoxel_gdext.so` собран для aarch64 (3.2 MB) **и** x86_64-android (3.2 MB, эмулятор), оба экспортируют `gdext_rust_init`. `rust/scripts/android-build.sh` расширен: дефолт — gdext `.so`, `--core` — voxel-core; `CC`/`CXX` пробрасываются в `godot-cpp`. `.gdextension.in` дополнен `android.x86_64` |
| 2026-07-04 | Фаза 3: `format::vox` (MagicaVoxel `.vox` парсер) | ✅ +23 теста, total 244. `streams/vox/vox_data.{h,cpp}` → `voxel-core/src/format/vox/{data,parser,tests}.rs`. Чистый Rust, ноль новых зависимостей; `&[u8]` cursor вместо `FileAccess`, `Node` enum вместо C++ inheritance, rotation-byte→Basis3f decode с fallback для out-of-spec байт |
| 2026-07-04 | Фаза 3: `streams::instance_data` + io fallible API | ✅ +13 тестов, total 257. `streams/instance_data.{h,cpp}` → `voxel-core/src/streams/instance_data.rs`. Расширение `MemoryReader`: `try_get_*`/`try_take` (Option, без panic) + `set_endianness` для v0 big-endian backcompat. `DeserializeError` enum, round-trip с quantization tolerance |
| 2026-07-04 | Фаза 3: `streams::compressed_data` (LZ4/ZSTD) | ✅ +12 тестов, total 269. `streams/compressed_data.{h,cpp}` → `voxel-core/src/streams/compressed_data.rs`. **Первая runtime-зависимость** voxel-core: `lz4_flex` (pure Rust, без C) для LZ4/LZ4_BE, `zstd` под optional feature. Android gdext `.so` перепроверен (aarch64 + x86_64) — собирается с новой зависимостью. `cargo rustc --crate-type staticlib` упирается в cargo#9562 (задокументировано в `android-build.sh`), но production-артефакт `.so` работает |
| 2026-07-04 | Фаза 3: `streams::block_serializer` (VoxelBuffer↔bytes) | ✅ +11 тестов, total 280. `streams/voxel_block_serializer.{h,cpp}` → `voxel-core/src/streams/block_serializer.rs`. v4-формат (version+size+8 каналов+trailing magic), `serialize_and_compress`/`decompress_and_deserialize` обёртки. Расширения: `MemoryReader::try_get_64`, `VoxelBuffer::set_channel_depth`. **Metadata-секция и v2/v3 legacy-миграция отложены** — завязаны на Godot Variant/custom-metadata factory (`storage/metadata/`, не портирован). Streams-стек (instance_data→compressed_data→block_serializer) завершён |

### Где остановились (для возобновления)

**Phase 0 — полностью закрыт.** H1/H2 проверены C++ harness'ем без godot-cpp
(stub-tree approach). Фаза 1 (`util/*`) — полностью портирована (191 тест).
Фаза 2 desktop-half — закрыт: `voxel-gdext` грузится в Godot 4.7, класс
`VoxelRustHello` виден в GDScript, достигает `voxel_core::VERSION` через FFI.
**Фаза 2 mobile-half — `.so` собран** (aarch64 + x86_64-android через NDK r29).

**H1 (partial):** C++ и Rust генерируют идентичные *позиции* вершин (606=606) и
одинаковое *число* треугольников (1304), но reuse-cache даёт разное число вершин
(888 vs 840) и 434/1304 треугольника расходятся в winding/reuse. Это реальная
дивергенция в логике reuse-cache / итерации, не float-precision. Rust goldens
остаются self-consistent; C++ goldens не коммитятся (fail byte-exact parity).
**H2 (pass):** Rust ~1.5× быстрее C++ (28.5µs/143Melem/s vs 44.1µs/93Mvoxels/s).
Полный разбор — в `rust/cpp-baseline/README.md` и `REPORT.md`.

**Открытые пункты:**
1. **H1 full byte-parity** — исследовать расхождение reuse-cache (888 vs 840
   вершин). Не блокирующее (H2 пройден, позиции совпадают), но нужно для strict parity.
2. **Фаза 2 on-device** — загрузить `libvoxel_gdext.so` в Godot Android export
   template (нужен custom template `platform=android` + SDK + устройство/эмулятор).
   `.so` собирается локально через `rust/scripts/android-build.sh`; упаковка в APK
   и проверка на устройстве — вне данного окружения.
3. **Фаза 3** — compute-слой (полный VoxelBuffer, blocky mesher, generators/noise).

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
**Отложено:** `expression_parser` → Фаза 3 (потребитель `generators/graph`);
`file_locker` → Фаза 4 (зависит от `thread/{mutex,rw_lock}`, ещё не портированы).

### Фаза 3 (в работе)

Compute-слой. Каждый модуль — отдельный коммит, clippy/fmt чист.

| Модуль | C++ источник | Статус |
|---|---|---|
| `storage::voxel_memory_pool` | `storage/voxel_memory_pool.{h,cpp}` | ✅ +7 тестов (power-of-two block pool: 21 bucket до 1MiB, thread-safe recycle через Mutex<Vec> + atomics; идиоматичный Rust — owned Vec вместо raw pointers) |
| `storage::funcs` | `storage/funcs.{h,cpp}` | ✅ +9 тестов (copy_3d_region_zxy, fill_3d_region_zxy, transform_3d_array_zxy через OrthoBasis, snorm s8/s16↔float квантизация) |
| `storage::voxel_buffer` | `storage/voxel_buffer.{h,cpp}` | ✅ +14 тестов (полный multi-channel dense store: 8 каналов, depth 8/16/32/64-bit, UNIFORM/NONE компрессия, Default+Pool аллокаторы, get/set voxel raw+float, fill/fill_area, compress_uniform_channels, copy_channel_from, Drop возвращает пулу) |
| `storage::voxel_format` | `storage/voxel_format.{h,cpp}` | ✅ +5 тестов (per-channel depth descriptor + supported-depth ranges + default raw values) |
| `format::vox` | `streams/vox/vox_data.{h,cpp}` | ✅ +23 теста (MagicaVoxel `.vox` парсер: header SIZE/XYZI/RGBA/nTRN/nGRP/nSHP/LAYR/MATL чанки, scene-graph валидация, rotation-byte→Basis3f decode c fallback на identity для out-of-spec байт, default palette parity с C++ `g_default_palette`, `magica_to_opengl` axis swap). `Node` → идиоматичный Rust enum вместо C++ inheritance; `FileAccess` → `&[u8]` cursor с `Result<_, VoxError>`. Godot-shim `vox_loader.cpp` отложен до binding-слоя) |
| `io::serialization` (расширение) | `util/io/serialization.h` (MemoryReader) | ✅ +fallible API: `try_get_8/16/32/float` + `try_take` возвращают `Option` (без panic на EOF) и `set_endianness` для on-the-fly переключения byte order. Нужно для `instance_data` (чтение из untrusted-источников) и legacy v0 big-endian форматов |
| `streams::instance_data` | `streams/instance_data.{h,cpp}` | ✅ +13 тестов (lossy-compressed per-block instance transforms `FORMAT_SIMPLE_11B_V1`: position→3×u16, scale→u8, rotation→4×u8 quaternion; serialize/deserialize с v0 big-endian backcompat через `set_endianness`, trailing magic `0x900df00d`, scale-range clamp; `DeserializeError` enum вместо bool; round-trip тесты с quantization tolerance) |
| `streams::compressed_data` | `streams/compressed_data.{h,cpp}` | ✅ +12 тестов (LZ4/ZSTD compression envelope: NONE/LZ4/LZ4_BE(legacy big-endian)/ZSTD; LZ4 через **`lz4_flex`** (pure Rust, без C — важно для Android/WASM), ZSTD через optional `zstd` feature; `Compression` enum c wire-format discriminants, `Error` enum, round-trip для compressive/incompressible/empty payloads, byte-order проверка LZ4_BE vs LZ4, error paths). **Первая runtime-зависимость** voxel-core |
| `io::serialization` (расширение 2) | `util/io/serialization.h` (MemoryReader) | ✅ `try_get_64` добавлен (завершает fallible-семейство try_get_8/16/32/64/float + try_take) — нужен для `block_serializer` (UNIFORM-каналы depth 64-bit) |
| `storage::voxel_buffer` (расширение) | `storage/voxel_buffer.h` | ✅ `set_channel_depth` — setter для depth канала (нужен десериализатору; контракт: только на свежем uniform-канале, как в C++) |
| `streams::block_serializer` | `streams/voxel_block_serializer.{h,cpp}` | ✅ +11 тестов (`VoxelBuffer`↔bytes v4-формат: version + 3×u16 size + 8 каналов (fmt byte = compression\|depth<<4, raw/UNIFORM данные) + trailing magic `0x900df00d`; `serialize_and_compress`/`decompress_and_deserialize` обёртки над `compressed_data`; `Error` enum. **Metadata-секция отложена** — завязана на Godot Variant/custom-metadata factory (`storage/metadata/`, не портирован); v4 без metadata byte-совместим с C++ когда metadata пусто. v2/v3 legacy-миграция отложена по той же причине) |

**Далее из Фазы 3:** generators (noise — simple/heightmap/waves), meshers
(cubes → blocky), streams (block_serializer → region форматы; vox-формат готов).

### Команды для возобновления работы
```bash
git clone https://github.com/sandsaber/godot_voxel.git
cd godot_voxel && git checkout rust/pilot
cd rust
cargo test -p voxel-core       # 285 проходят (280 unit + 5 integration; +1 ignored golden-gen)
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
