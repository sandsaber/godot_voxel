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

### NDK note (Phase 2, not needed here)

This harness is host-built C++; it does not touch Android. The Android `.so`
finding (rustc LLVM 22 vs NDK r29 LLVM 21 skew, resolved by forcing `rust-lld`)
is documented in `REPORT.md` and the android build helper.
