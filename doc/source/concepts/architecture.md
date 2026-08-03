# Architecture

## The two-crate split

```
┌────────────────────────────────────────────────────────────┐
│ voxel-gdext  (Godot 4 GDExtension)                         │
│ 79 classes: #[func]/#[var] wrappers, type conversion only. │
│ The ONLY crate that depends on the `godot` crate.          │
└──────────────────────────┬─────────────────────────────────┘
                           │ delegates every call
┌──────────────────────────▼─────────────────────────────────┐
│ voxel-core  (pure Rust, engine-agnostic)                   │
│ math · storage · meshers · generators · streams · terrain  │
│ paging · tasks · edition · instancing · modifiers          │
│ cargo-testable without Godot; cross-compiles to Android.   │
└────────────────────────────────────────────────────────────┘
```

All logic and every hot path live in `voxel-core`; the binding converts Godot
types and forwards. New features belong in `voxel-core`, wrapped by a thin
`#[func]` if they need a Godot surface. This boundary is what keeps the core
unit-testable (≈800 tests run without Godot) and cross-compilable.

## The data pipeline

```
GraphGenerator (SDF/Curve/Noise/math/IO nodes)
   │  or  Waves / Flat / Noise / HeightmapNoise / Image (simple generators)
   ▼
VoxelData  (settings + per-LOD block maps + spatial locks)
   ▼
MeshBlockTask  (gathers a 3×3×3 voxel neighbourhood, fills gaps from the
   │            generator, then runs the mesher on a worker thread)
   ▼
VoxelMesher  ◂── TransvoxelMesher  (smooth SDF terrain, LOD transitions)
             ◂── CubesMesher       (greedy/simple colored cubes, palette)
             ◂── BlockyMesher      (voxel-model library + ambient occlusion)
   ▼
MesherOutput { surfaces, collision_surface }
   ▼
VoxelTerrainCore  (paging orchestrator: viewer updates → load / mesh /
   │               unload decisions, per-LOD)
   ▼
VoxelEngine  (volume/viewer registry, owns the threaded task runner)
```

The whole pipeline runs end-to-end in pure Rust and is covered by
integration tests. On the Godot side, `VoxelTerrain` owns a
`VoxelTerrainCore`, feeds viewer positions each frame, and uploads finished
meshes as `ArrayMesh` resources into child `MeshInstance3D` nodes.

## Key concepts

- **Blocks**: voxel data is stored in 16³ blocks. `VoxelData` keeps one map
  per LOD level.
- **Channels**: each voxel has up to 8 channels (`type`, `sdf`, `color`,
  `indices`, `weights`, …). Smooth terrain uses the SDF channel; blocky
  terrain uses `type`/`color`.
- **Streams** produce/consume blocks: generators create data on demand,
  memory/region streams persist it. A terrain pairs a stream with a
  generator (the generator fills blocks the stream has no data for).
- **Viewers** declare where detail is needed. The paging orchestrator loads
  blocks around viewers, meshes them, and unloads what left range.
- **Mesher** choice defines the look: transvoxel = smooth, cubes = Minecraft
  style, blocky = model-based. Assign one via the `VoxelTerrain.mesher`
  property.

## Concurrency model

- Block loads/meshes run on a `ThreadedTaskRunner` thread pool.
- `VoxelData` is shared via worker handles: a settings lock + per-LOD map
  `RwLock`s + a `SpatialLock3D` that serializes edits touching the same
  region. Lock ordering is unit-tested.
- The graph generator compiles its graph once and shares it through an
  `Arc` across worker threads.

## Class naming

Godot classes keep their **canonical upstream names** (`VoxelBuffer`,
`VoxelTerrain`, …) via `#[class(rename = …)]`; the Rust structs carry a `GD`
suffix internally (`VoxelBufferGD`). The three `ZN_`-prefixed classes
(`ZN_FastNoiseLite`, `ZN_SpotNoise`, `ZN_Curve`) avoid collisions with Godot
builtins, mirroring upstream.
