# EDIT-1: transactional procedural edits

## Goal

Make a single voxel edit preserve the procedural contents of a previously
unloaded block and make the mutation persistable without a gap between writing
the voxel and marking its block dirty.

## Scope

The change applies to the headless `SharedVoxelData` edit path. It does not
add batch editing, change persistence scheduling, or redesign the map into
per-block shards.

## Design

Add one transactional public operation for a single voxel edit. It owns this
sequence:

1. Acquire the affected `SpatialLock3D` write region.
2. Look up the containing data block. If absent, snapshot the generator and
   materialize the complete procedural block outside the map lock.
3. Reacquire the map write lock and insert the materialized block only if a
   concurrent path has not inserted one already. If it has, use that existing
   block instead.
4. Change the requested voxel and set the block's modified and edited state
   under the same spatial guard before releasing it.

Generator callbacks must remain outside the map lock. The spatial guard stays
live across materialization and insertion so an overlapping edit cannot expose
an unmaterialized/default block. Existing low-level helpers may remain for
internal callers, but terrain-facing single-voxel edits use the new operation.

## Failure handling

If there is no generator, materialize the existing format default exactly as
today. A generator error or unavailable generator leaves the map unchanged and
returns a typed error; it must never insert a partial/default replacement for a
procedural block.

## Tests

- Editing one voxel in an absent procedural block keeps generator-derived
  values at neighbouring coordinates.
- The edit is already marked modified/edited when the operation returns, so
  an immediate unview produces a save candidate.
- A deterministic competing-insert case retains the already-resident block
  instead of overwriting it with a stale materialization.
- Existing non-procedural/default-buffer edit behaviour remains covered.

## Non-goals

- Batch `EditSession` API.
- Per-block map sharding or copy-on-write snapshots.
- Persistence retries and ordering, which are covered by the Wave 0 save
  journal work.
