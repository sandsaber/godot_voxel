# GRAPH-1: SdfSmoothSubtract parity

## Goal

Make Rust `SdfSmoothSubtract` use the same operand order as the C++ voxel
graph node, so generated SDF signs and resulting terrain topology match the
reference implementation.

## Scope

This stage changes only the smooth-subtract runtime evaluation and its tests.
It does not add multi-output graph ports or change Distance, Normalize, Remap,
or Divide; those are separate approved stages.

## Design

`NodeKind::SdfSmoothSubtract { a, b, smoothness }` will call
`sdf_smooth_subtract(a, b, smoothness)` for positive smoothness, matching the
existing hard-subtract fallback and C++ `nodes/sdf.h` contract. The current
call swaps `a` and `b`, which changes the signed-distance result.

## Tests

- Add a direct graph regression using constants `a=-0.2`, `b=0.4`, and
  `smoothness=1.0`; it must evaluate to approximately `-0.04`.
- Add a zero-smoothness regression proving the smooth node follows the same
  operand order as hard subtraction.
- Keep existing graph-runtime tests green.

## Non-goals

- C++ asset import/export schema changes.
- Multi-output ports.
- Other graph-node parity fixes.
