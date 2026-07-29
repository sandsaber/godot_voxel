Voxel Tools for Godot
=========================

A voxel terrain engine for Godot Engine 4, **fully ported from C++ to Rust**.

This fork is a **pure Rust GDExtension** — no C++ module code remains. The
engine core (`voxel-core`) is engine-agnostic and fully unit-testable; the
thin Godot binding (`voxel-gdext`) exposes 80 functional classes via
`#[func]` methods. Loads in Godot 4.7+.

![Blocky screenshot](doc/source/images/blocky_screenshot.webp)
![Smooth screenshot](doc/source/images/smooth_screenshot.webp)

Features
---------------------------

- Realtime 3D terrain editable in-game (overhangs, tunnels, creation/destruction)
- Polygon-based: voxels are transformed into chunked meshes via the Transvoxel algorithm
- Godot physics integration + fast Minecraft-like collisions
- Infinite terrains via multi-LOD paging (LodOctree + transition cells)
- Voxel data streaming (memory, region files, generators)
- Minecraft-style blocky terrain with baked ambient occlusion
- Smooth terrain with level of detail (Transvoxel + SINGLE_S4 texturing)
- Procedural graph generator (24+ node types, expression nodes, image lookups)
- Voxel instancing system (scatter foliage, rocks on surfaces)
- **Pure Rust** — cross-compiles to Android (aarch64/x86_64), iOS, macOS

Building
---------------

```bash
cd rust
cargo build -p voxel-gdext --release
```

This produces the `.so`/`.dylib`/`.dll` GDExtension library. Copy the
`.gdextension` file from `rust/voxel-gdext/voxel_gdext.gdextension.in` into
your Godot project and point it at the built library.

Testing
---------------

```bash
cd rust
cargo test -p voxel-core -p voxel-gdext    # 795 unit + 674 parity + 5 integration
cargo clippy --workspace --all-targets      # clean
```

Project structure
---------------

```
rust/
├── voxel-core/          # Engine-agnostic Rust core (all logic)
│   ├── src/             # 795 unit tests
│   └── tests/           # 674 parity tests (mirrors C++ test suite)
├── voxel-gdext/         # Godot GDExtension binding (80 classes)
│   ├── src/             # #[func] methods delegating to voxel-core
│   └── smoke_test/      # Godot 4.7 project + VoxelGeneratorGraph addon
├── cpp-baseline/        # C++ parity harness (reference data generation)
├── tsan/                # ThreadSanifier tests
└── fuzz/                # cargo-fuzz targets
```

Migration status
---------------

All milestones closed: **M1 ✅ M2 ✅ M3 ✅ M4 ✅**

| Milestone | Description |
|---|---|
| M1 | Code review debt closed (TSan, typed storage, mesher perf, graph compile, fuzz) |
| M2 | Phase 4 multi-LOD paging GO (LodOctree + transition cells) |
| M3 | 80/80 Godot classes functional, Godot 4.7 GDExtension loads |
| M4 | Full C++ parity: 674 parity tests + 9 ported features (box_blur, texturing, FastNoise2, etc.) |

See [`rust/STATUS.md`](rust/STATUS.md) for details.

Documentation
---------------

- [Migration plan](MIGRATION_PLAN.md)
- [Rust port status](rust/STATUS.md)
- [Audit report](rust/AUDIT.md)
- [Phase 0 pilot report](REPORT.md)
- [Original docs](https://voxel-tools.readthedocs.io/en/latest/)

Credits
---------------

Originally developed by [Zylann](https://github.com/Zylann/godot_voxel).
Rust port by the community. See the supporter list in the original project.
