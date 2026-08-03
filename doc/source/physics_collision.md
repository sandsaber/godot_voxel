# Physics & collision

!!! note "Status: partially implemented"
    - **Works:** per-block trimesh collision for `VoxelTerrain` (via Godot's
      built-in shape generation) and DDA voxel raycasting on `VoxelTerrain`.
    - **Non-upstream semantics:** `VoxelBoxMover` and `VoxelAStarGrid3D` exist
      but behave differently from the original C++ module — details below.
    - No dedicated physics integration (e.g. Rapier) exists; collision relies
      on Godot's own physics bodies and shapes.

## Generating collision shapes

Enable collision on the terrain, and every mesh block gets a trimesh collision
shape as it is uploaded: internally each block's `MeshInstance3D` has Godot's
built-in `create_trimesh_collision()` called on it, which spawns a
`StaticBody3D` with a concave polygon shape per block.

```gdscript
var terrain := VoxelTerrain.new()
terrain.set_generate_collision(true)      # before the terrain loads blocks
terrain.set_generator(VoxelGeneratorFlat.new())
add_child(terrain)
```

Notes:

- The flag is read **when a block is uploaded**. Enable it before the terrain
  starts loading (e.g. before `add_child`); blocks uploaded earlier get no
  collision until they are re-meshed or reloaded.
- Collision is generated from the same transvoxel mesh that is rendered, so it
  matches the visible surface.
- `get_generate_collision()` reports the current flag.

## Raycasting against terrain

`VoxelTerrain.raycast()` traverses voxels along the ray with the Amanatides &
Woo DDA algorithm (exact voxel traversal, no stepping artifacts) and reports
the first voxel whose SDF is solid:

```gdscript
# Signature: raycast(origin_x, origin_y, origin_z, dir_x, dir_y, dir_z, max_distance)
var r := $Terrain.raycast(0.0, 200.0, 0.0, 0.0, -1.0, 0.0, 512.0)
if r.size() == 4 and r[3] > 0.0:
    print("Hit terrain at voxel ", Vector3i(r[0], r[1], r[2]))
```

- Returns a `PackedFloat32Array` `[x, y, z, hit]` with the hit **voxel
  position**; `hit` is `1.0` on hit, `0.0` otherwise. An empty array means
  the terrain core is not initialised.
- The direction is normalised internally; `max_distance` is in voxels.
- A hit is registered where SDF < 0 (inside solid).

`VoxelRaycastResult` (`hit_x/y/z`, `prev_x/y/z`, `distance`, `normal_x/y/z`,
plus `did_hit()` and `get_hit_position()`) and `VoxelBlockRaycastResult`
(`voxel_id`, `hit_x/y/z`, same two methods) exist as plain data containers you
can fill yourself; no terrain method currently returns them.

## VoxelBoxMover

!!! warning "Differs from the upstream C++ module"
    In the original module `VoxelBoxMover` helped move a box-shaped character
    against voxel collision. In this port it is a **buffer editing tool**: it
    stamps a solid box along a straight path into a buffer's Type channel.

| Member | Kind | Notes |
|---|---|---|
| `box_size` | property | Half-size of the stamped box in voxels. Default `2.0`. |
| `carve_path(buffer, origin_x, target_x, target_y, target_z)` | method | From the node's position to `(target_x, target_y, target_z)`, stamps a solid box at every integer step into a `VoxelBuffer`'s Type channel (channel 0). `origin_x` offsets the stamps into the buffer's local space. Returns the number of steps stamped, `-1` if `buffer` is not a `VoxelBuffer`. |

```gdscript
var mover := VoxelBoxMover.new()
mover.box_size = 2.0
mover.position = Vector3(4, 4, 4)
add_child(mover)

var buffer := VoxelBuffer.new()
buffer.create(32, 16, 32)
print(mover.carve_path(buffer, 0, 28, 8, 28))
```

## VoxelAStarGrid3D

!!! warning "Differs from the upstream C++ module"
    Upstream provides A* pathfinding over voxel terrain. The voxel-core
    pathfinding engine is not ported yet; this class is a **walkability
    classifier over a buffer** — no path search.

| Method | Notes |
|---|---|
| `is_walkable(buffer, x, y, z)` | Ground-walking semantics: the cell itself must be air and the cell below solid (Type channel). `false` for out-of-bounds cells or a non-`VoxelBuffer` argument. |
| `count_walkable(buffer)` | Number of walkable cells in the whole buffer (`-1` if not a `VoxelBuffer`). |

```gdscript
var grid := VoxelAStarGrid3D.new()
var buffer := VoxelBuffer.new()
buffer.create(16, 8, 16)
for x in 16:
    for z in 16:
        buffer.set_voxel(x, 2, z, 0, 1)   # solid floor at y=2
print(grid.is_walkable(buffer, 8, 3, 8))  # air above the floor → true
print(grid.count_walkable(buffer))
```
