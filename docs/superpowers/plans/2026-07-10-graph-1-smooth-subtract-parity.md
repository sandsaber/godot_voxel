# GRAPH-1: SdfSmoothSubtract Parity Implementation Plan

> **Status:** completed and reconciled on 2026-07-24 (`d3569f34`; lazy and compiled evaluator paths covered).
>
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Match C++ operand order for `SdfSmoothSubtract` graph evaluation.

**Architecture:** Keep the existing scalar graph runtime and public graph schema. First add graph-level parity tests, then replace the swapped operands in the positive-smoothness branch; the zero-smoothness branch already delegates to correctly ordered hard subtraction.

**Tech Stack:** Rust 1.96.1, voxel-core graph runtime, cargo test.

## Global Constraints

- Change only `SdfSmoothSubtract` evaluation and its runtime tests.
- Positive smoothness uses `sdf_smooth_subtract(a, b, smoothness)`.
- Zero smoothness must retain `sdf_subtract(a, b)`.
- Do not modify the pending `rust/AUDIT.md` or unrelated plan files.

---

### Task 1: Lock in and correct smooth-subtract parity

**Files:**

- Modify: `rust/voxel-core/src/generators/graph/runtime.rs:659-670, tests`

**Interfaces:**

- Consumes: `NodeKind::Constant`, `NodeKind::SdfSmoothSubtract`, `NodeKind::OutputSdf`, `Graph::generate`.
- Produces: C++-compatible SDF output for smooth and zero smoothness.

- [ ] **Step 1: Write failing graph parity tests**

  Add `smooth_subtract_graph(a, b, smoothness) -> f32` test helper that builds two constants, a `SdfSmoothSubtract`, and `OutputSdf`, evaluates one sample, and returns `outputs[0].1[0]`. Add:

  ```rust
  #[test]
  fn smooth_subtract_node_matches_cpp_operand_order() {
      assert!((smooth_subtract_graph(-0.2, 0.4, 1.0) - -0.04).abs() < 1e-5);
  }

  #[test]
  fn smooth_subtract_node_uses_hard_subtract_at_zero_smoothness() {
      assert!((smooth_subtract_graph(-0.2, 0.4, 0.0)
          - crate::math::sdf::sdf_subtract(-0.2, 0.4)).abs() < 1e-5);
  }
  ```

- [ ] **Step 2: Verify RED**

  Run: `cargo test --manifest-path rust/Cargo.toml -p voxel-core smooth_subtract_node`

  Expected: the positive-smoothness test fails because the current runtime calls `sdf_smooth_subtract(b, a, s)`; zero-smoothness passes.

- [ ] **Step 3: Make the minimal runtime correction**

  In the `s > 1e-4` branch replace:

  ```rust
  crate::math::sdf::sdf_smooth_subtract(b, a, s)
  ```

  with:

  ```rust
  crate::math::sdf::sdf_smooth_subtract(a, b, s)
  ```

- [ ] **Step 4: Verify GREEN and regression suite**

  Run: `cargo test --manifest-path rust/Cargo.toml -p voxel-core smooth_subtract_node && cargo test --manifest-path rust/Cargo.toml -p voxel-core generators::graph::runtime::tests`

  Expected: all focused and graph-runtime tests pass.

- [ ] **Step 5: Verify formatting and commit**

  Run: `cargo fmt --manifest-path rust/Cargo.toml --all --check`

  ```bash
  git add rust/voxel-core/src/generators/graph/runtime.rs
  git commit -m "fix(rust): match smooth subtract graph parity"
  ```

## Plan Review

- Spec coverage: the single task proves the C++ golden vector and zero-smooth fallback, then changes exactly the swapped call.
- Type consistency: tests use existing `Graph` and `NodeKind` APIs; runtime signature is unchanged.
- Scope: no port-schema, graph-node, or audit-document changes are included.
