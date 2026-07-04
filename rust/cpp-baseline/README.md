# `cpp-baseline` — C++ reference for parity & perf comparison

This directory produces reference data from the **real upstream C++** godot_voxel
sources, to validate the Rust port against. Phase 0 needs a trustworthy C++
reference for two of the four GO/NO-GO hypotheses:

- **H1 (equivalence):** the Rust mesher must produce the same mesh as C++.
- **H2 (performance):** the Rust mesher must not be slower than C++.

## What works today (table parity)

`dump_tables.cpp` compiles the *real* `meshers/transvoxel/transvoxel_tables.cpp`
against an empty stub for its only include (`util/errors.h`) and prints the three
regular-cell lookup tables. The output lands in
`voxel-core/tests/golden/transvoxel_tables_cpp.txt`, and the Rust test
`transvoxel_tables_parity` asserts the ported tables are **byte-for-byte identical**.

Table parity is necessary but not sufficient for full mesh parity: the lookup
backbone is proven identical, so any future mesh difference can only come from
the small vertex-interpolation / reuse-cache logic.

Regenerate after pulling upstream changes:

```sh
./build.sh                 # default g++
CXX=clang++ ./build.sh     # override compiler
```

## What still needs doing (full mesh parity + perf)

The mesher body — `build_regular_mesh` in `transvoxel.cpp` — depends on Godot
types (`Vector3`, `Vector3i`, `Span`, `FixedArray`, `Math`, …) via the module's
`util/godot/` shim, which only has two backends: `ZN_GODOT` (Godot source) and
`ZN_GODOT_EXTENSION` (godot-cpp). There is no standalone build path, so compiling
the real mesher requires one of:

1. **godot-cpp (recommended).** Vendor `godot-cpp` (git submodule or fetch), build
   it with `ZN_GODOT_EXTENSION` defined, link a tiny harness that calls
   `build_regular_mesh<float>` (TEXTURES_NONE) on the same SDF sphere used by the
   Rust golden, and dump the mesh in the existing `GoldenMesh` JSON schema
   (`generator = godot_voxel-cpp`). Then regenerate `voxel-core/tests/golden/*.json`
   from C++ and the existing `matches_golden_*` tests become true H1 parity.
   The same harness times the C++ run for the H2 perf comparison.
2. **Godot source.** Heavier; only worth it if godot-cpp lags a needed API.

Effort estimate: ~1 focused session to vendor godot-cpp, build it, write the
mesh-dumping harness, and wire the JSON into the existing golden machinery. The
golden schema and comparator are already done, so this is purely a C++ build task.

## DONE — mesh harness (no godot-cpp needed)

The lighter stub approach won: the inner `build_regular_mesh<float, NullProcessor>`
template (transvoxel.cpp lines 1–602) only needs a handful of Godot types
(`Vector3i`, `Math::*`, `StdVector`, `Cube::`), all provided as tiny stubs. No
godot-cpp vendoring, no Godot source.

**`build_mesh.sh`** assembles a stub header tree (shadowing the godot-shim
headers) + trims the copied `transvoxel.cpp` to the inner template only
(dropping the transition mesh + `VoxelBuffer` dispatcher that pull in heavy
Godot APIs we don't exercise), then compiles **`dump_mesh.cpp`** which:

1. Fills a `std::vector<float>` with the SDF sphere (identical to the Rust
   `SphereInput::new` — same ZXY layout, same radius/inner/padding).
2. Calls the real `build_regular_mesh<float, NullProcessor>` template.
3. Emits a `GoldenMesh` JSON (`generator: "godot_voxel-cpp"`).
4. Times 50 runs for the H2 baseline (timing to stderr).

```sh
./build_mesh.sh                 # 16-sphere: JSON→stdout, timing→stderr
./build_mesh.sh --regenerate    # rewrite both golden JSONs from C++
```

## Results (16-sphere, inner=16, radius=6.0)

### H2 (performance) — PASS

| impl | time/run | throughput |
|------|----------|------------|
| Rust (criterion) | **28.5 µs** | **143 Melem/s** |
| C++ harness | 44.1 µs | 93 Mvoxels/s |

The Rust port is **≥1.5× faster** than the C++ reference at this size (the C++
harness was built `-O2` host-native; the Rust criterion bench used the workspace
`release` profile with fat LTO). H2 target (perf ≥ C++ −15%) is comfortably met.
(The C++ number is a lower bound on the real engine's perf since the harness
goes through a plain `std::vector<float>` rather than `VoxelBuffer`'s compressed
path; the engine may be faster. But it establishes that Rust is not slower.)

### H1 (equivalence) — PARTIAL: positions match, indices/winding differ

The harness reveals the Rust port is **algorithmically correct but not
byte-identical** to C++:

| metric | C++ | Rust | match? |
|--------|-----|------|--------|
| unique vertex positions | 606 | 606 | ✅ identical sets |
| triangle count | 1304 | 1304 | ✅ |
| index_count | 3912 | 3912 | ✅ |
| **vertex_count** | **888** | **840** | ❌ (vertex reuse cache differs) |
| triangles-as-sets | 1304 | 1304 | ❌ 434 differ (winding/vertex choice) |

**Interpretation:** the two meshers generate exactly the same vertex *positions*
and the same *number* of triangles, but differ in (a) how the vertex reuse cache
collapses shared positions (888 vs 840 vertices), and (b) the winding order /
which reused vertex a triangle picks (434/1304 triangles differ as ordered
position-triples). This is a real divergence in the reuse-cache and cell-iteration
logic, not a float-precision issue. The committed Rust goldens remain
self-consistent (`generator: "voxel-core-rust"`); the C++ goldens from
`--regenerate` are NOT committed because they would *fail* the byte-exact
`matches_golden_*` tests. Investigating the reuse-cache divergence is the next
step toward full H1 byte-parity.

### NDK note (Phase 2, not needed here)

This harness is host-built C++; it does not touch Android. The Android `.so`
finding (rustc LLVM 22 vs NDK r29 LLVM 21 skew, resolved by forcing `rust-lld`)
is documented in `REPORT.md` and the android build helper.
