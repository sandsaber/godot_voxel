# Roadmap

Big features that remain after the C++ → Rust migration. Each item is
independently trackable — reference it in commits/PRs as `R1`, `R2`, …
Statuses: ⬜ not started · 🟡 in progress · ✅ done. Detailed rationale and
the full parity matrix live in [doc/source/status.md](doc/source/status.md).

## R1 — Blocky terrain end-to-end ⬜

Attach a baked model library to `VoxelMesherBlocky` so blocky terrain renders
on `VoxelTerrain`.

- [ ] Expose `VoxelBlockyLibrary` baking through the binding (models →
      `BakedLibrary` + `bake_library`)
- [ ] Let `VoxelMesherBlocky` carry a library resource into the terrain
      pipeline
- [ ] Smoke test: type-channel generator + blocky mesher renders visible
      blocks

## R2 — VoxelLodTerrain paging & rendering ⬜

Replace the current facade with a real multi-LOD node (upstream behavior):
octree-driven paging, streaming, rendering with LOD transitions.

- [ ] Wire `LodOctree` decisions to stream load/save in the node
- [ ] Render LOD blocks + transition meshes (core meshers already support it)
- [ ] Viewer-driven subdivision/joining in `_process`

## R3 — Multiplayer / areas ⬜

- [ ] Port `VoxelAreaFinder` (area sync primitives)
- [ ] Define the replication boundary for voxel edits/block data

## R4 — Terrain editing tools ⬜

Upstream `VoxelTool` surface on real terrain.

- [ ] `VoxelToolTerrain` backed by `VoxelTerrainCore` edits (sphere/box)
- [ ] Smooth and paste modes

## R5 — Instancing rendering ⬜

- [ ] `VoxelInstanceBlock` + per-block instance streaming
- [ ] MultiMesh output from scatter results (currently counts only)

## R6 — Graph editor parity ⬜

- [ ] Parse graph JSON back into nodes (`set_graph_json` round-trip)
- [ ] Wire `ExpressionNode` / `Image2D` into the graph runtime
- [ ] Visual editor (GDScript GraphEdit addon or native)

## R7 — Streams & metadata ⬜

- [ ] Block metadata section (needs a Variant codec; also unblocks v2/v3
      legacy migration)
- [ ] `VoxelStreamRegionFiles` settings surface (region/sector size, channel
      depths, rotation, file conversion)

## R8 — CI rework ⬜

The old scons-based workflows are dead after the C++ removal; `rust.yml` is
manual-only.

- [ ] Automatic Rust CI on push/PR (build + test + clippy + fmt)
- [ ] Remove or replace the legacy C++ workflows

## Deferred by design (no ETA)

GPU compute path / detail rendering / shaders, SQLite streams, multipass
generator, Rapier physics — intentionally out of scope to keep `voxel-core`
pure-Rust and cross-compilable.
