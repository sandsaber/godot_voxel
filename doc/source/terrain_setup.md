# Terrain quickstart

This page shows how to get visible voxel terrain into a scene using the two
core nodes of the Rust port:

- **`VoxelTerrain`** — a `Node3D` that pages in voxel data, meshes it and
  uploads the meshes as child `MeshInstance3D` nodes.
- **`VoxelViewer`** — a `Node3D` that marks a position the terrain streams
  blocks around.

Both classes are editor-compatible (`tool`) and run in the editor viewport as
well as at runtime.

!!! note "Status: partially implemented"
    What works today:

    - Paging around `VoxelViewer` children, with single- or multi-LOD output.
    - Generators: `VoxelGeneratorWaves`, `VoxelGeneratorFlat`,
      `VoxelGeneratorNoise`, `VoxelGeneratorHeightmap`, `VoxelGeneratorImage`,
      `VoxelGeneratorGraph` (see [generators](generators.md)).
    - Mesher selection via the `mesher` property: transvoxel (default),
      cubes, blocky (see [meshers](meshers.md)).
    - Streams: `VoxelStreamMemory`, `VoxelStreamRegionFiles`
      (see [streams](streams.md)).
    - Per-block trimesh collision, voxel SDF editing, SDF raycasts
      (see [physics & collision](physics_collision.md)).

    Known gaps:

    - Blocky terrain needs a baked model library the binding cannot attach
      yet, so `VoxelMesherBlocky` renders nothing on terrain for now.
    - Terrain bounds are fixed at 2048³ voxels, from `(-512, -512, -512)` to
      `(1535, 1535, 1535)` (queryable via `get_bounds()`).
    - Only **direct children** of `VoxelTerrain` are picked up as viewers.

## Minimal setup in GDScript

The terrain builds its paging core in `_ready()`, so the generator, stream and
LOD count must be assigned **before the node enters the tree**:

```gdscript
extends Node3D

func _ready() -> void:
    var terrain := VoxelTerrain.new()
    terrain.name = "Terrain"

    var generator := VoxelGeneratorHeightmap.new()
    generator.seed = 1234
    generator.frequency = 0.01
    generator.height_range = 80.0
    terrain.set_generator(generator)

    add_child(terrain)

    var viewer := VoxelViewer.new()
    viewer.view_distance = 128          # voxels
    viewer.position = Vector3(0, 100, 0)
    terrain.add_child(viewer)           # must be a direct child
```

If no generator is assigned, the terrain defaults to a waves generator
(pattern size 128, height range 60). If no stream is assigned, terrain is
generated on the fly and never persisted. Setting `lod_count` to 2 or more
without a stream installs an internal `VoxelStreamMemory` automatically.

## VoxelTerrain API

| Member | Kind | Notes |
|---|---|---|
| `stream` | exported property | `VoxelStreamMemory` or `VoxelStreamRegionFiles`; also settable via `set_stream()` / `get_stream()`. Any other resource is rejected with an error. |
| `set_generator(res)` / `get_generator()` | method | Generator resource; assign before `_ready`. |
| `set_lod_count(n)` / `get_lod_count()` | method | `1` = single-LOD, `2+` = multi-LOD. Clamped to `1..24`. Set before `_ready`. |
| `set_material_override(mat)` / `get_material_override()` | method | Material applied to every mesh block. |
| `set_generate_collision(b)` / `get_generate_collision()` | method | Enables per-block trimesh collision (see [physics](physics_collision.md)). |
| `set_voxel_sdf(x, y, z, value)` / `get_voxel_sdf(x, y, z)` | method | Read/write single SDF voxels; writes re-mesh the block (see [editing](editing.md)). |
| `raycast(ox, oy, oz, dx, dy, dz, max_distance)` | method | SDF ray march, returns `[x, y, z, hit]`. |
| `get_mesh_block_count()` | method | Number of uploaded mesh blocks. |
| `get_bounds()` | method | `[min_x, min_y, min_z, size_x, size_y, size_z]`. |
| `get_statistics()` | method | `VoxelTerrainStats` snapshot (`blocks_loaded`, `blocks_unloaded`, `meshes_built`, `meshes_dropped`), or `null` before `_ready`. |
| `get_version()` | method | voxel-core version string. |

Blocks are 16³ voxels. Each mesh block becomes a child `MeshInstance3D` named
`mesh_X_Y_Z` positioned at the block's world origin.

## VoxelViewer API

| Member | Kind | Notes |
|---|---|---|
| `view_distance` | property | View radius in voxels (horizontal and vertical). Default `96`. |

Add one viewer per camera/player. The terrain re-reads viewer positions every
frame, so viewers can move freely. Attaching the viewer to the player node is
the typical setup:

```gdscript
# Inside your player script:
func _ready() -> void:
    var viewer := VoxelViewer.new()
    viewer.view_distance = 160
    add_child(viewer)
```

## Diagnostics

```gdscript
func _process(_delta: float) -> void:
    var stats = $Terrain.get_statistics()
    if stats != null:
        print("blocks: %d loaded, meshes: %d built, visible: %d" % [
            stats.blocks_loaded, stats.meshes_built,
            $Terrain.get_mesh_block_count()])
```
