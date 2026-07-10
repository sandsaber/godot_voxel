# Wave 0A Safe Vector API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove reachable unchecked unsafe from public `Vector3T::get/set` and `Vector4T::get/set`.

**Architecture:** Keep the current vector API and panic behavior aligned with `Index`/`IndexMut`. Invalid indices must panic in both debug and release builds instead of reaching `core::hint::unreachable_unchecked()`.

**Tech Stack:** Rust, `voxel-core`, unit tests in existing math modules.

## Global Constraints

- Preserve existing vector struct layout and public function signatures.
- Do not change `Index` or `IndexMut` semantics except to keep panic messages consistent.
- Add tests before implementation changes.
- Do not touch `rust/AUDIT.md`; it is user-owned working-tree state.

---

## File Structure

- Modify: `rust/voxel-core/src/math/vector3.rs`
  - Add unit tests for valid and invalid `get/set`.
  - Replace `unreachable_unchecked()` fallback arms in public methods with explicit panics.
- Modify: `rust/voxel-core/src/math/vector4.rs`
  - Add unit tests for valid and invalid `get/set`.
  - Replace `unreachable_unchecked()` fallback arms in public methods with explicit panics.

### Task 1: Vector3 Public Accessors Panic Safely

**Files:**
- Modify: `rust/voxel-core/src/math/vector3.rs`
- Test: `rust/voxel-core/src/math/vector3.rs`

**Interfaces:**
- Consumes: existing `pub fn get(&self, i: usize) -> T` and `pub fn set(&mut self, i: usize, v: T)`.
- Produces: same signatures; invalid `i >= 3` panics with `Vector3 index out of range`.

- [ ] **Step 1: Write the failing tests**

Append this test module near the bottom of `rust/voxel-core/src/math/vector3.rs` if the file has no existing `#[cfg(test)]` module; if one exists, add the tests inside it.

```rust
#[cfg(test)]
mod tests {
    use super::Vector3T;

    #[test]
    fn public_get_returns_vector3_components() {
        let v = Vector3T::new(10, 20, 30);

        assert_eq!(v.get(0), 10);
        assert_eq!(v.get(1), 20);
        assert_eq!(v.get(2), 30);
    }

    #[test]
    fn public_set_updates_vector3_components() {
        let mut v = Vector3T::new(1, 2, 3);

        v.set(0, 10);
        v.set(1, 20);
        v.set(2, 30);

        assert_eq!(v, Vector3T::new(10, 20, 30));
    }

    #[test]
    #[should_panic(expected = "Vector3 index out of range")]
    fn public_get_panics_for_out_of_range_vector3_index() {
        let v = Vector3T::new(1, 2, 3);
        let _ = v.get(3);
    }

    #[test]
    #[should_panic(expected = "Vector3 index out of range")]
    fn public_set_panics_for_out_of_range_vector3_index() {
        let mut v = Vector3T::new(1, 2, 3);
        v.set(usize::MAX, 4);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p voxel-core math::vector3::tests::public_get_panics_for_out_of_range_vector3_index --locked`

Expected: FAIL or abort from the existing unchecked fallback in release/test execution when invalid `get` reaches `unreachable_unchecked()`. If debug assertions trigger first, the failure still demonstrates the public method is not using the intended stable panic message.

- [ ] **Step 3: Write minimal implementation**

Replace `Vector3T::get` and `Vector3T::set` with:

```rust
    /// Index access with runtime bounds check. Mirrors `operator[]`.
    #[inline]
    pub fn get(&self, i: usize) -> T {
        match i {
            0 => self.x,
            1 => self.y,
            2 => self.z,
            _ => panic!("Vector3 index out of range"),
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: T) {
        match i {
            0 => self.x = v,
            1 => self.y = v,
            2 => self.z = v,
            _ => panic!("Vector3 index out of range"),
        }
    }
```

- [ ] **Step 4: Run focused vector3 tests**

Run: `cargo test -p voxel-core math::vector3::tests --locked`

Expected: PASS.

- [ ] **Step 5: Search for remaining unchecked fallback in vector3**

Run: `rg -n "unreachable_unchecked" rust/voxel-core/src/math/vector3.rs`

Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add rust/voxel-core/src/math/vector3.rs
git commit -m "fix(rust): make vector3 accessors panic safely"
```

### Task 2: Vector4 Public Accessors Panic Safely

**Files:**
- Modify: `rust/voxel-core/src/math/vector4.rs`
- Test: `rust/voxel-core/src/math/vector4.rs`

**Interfaces:**
- Consumes: existing `pub fn get(&self, i: usize) -> T` and `pub fn set(&mut self, i: usize, v: T)`.
- Produces: same signatures; invalid `i >= 4` panics with `Vector4 index out of range`.

- [ ] **Step 1: Write the failing tests**

Append this test module near the bottom of `rust/voxel-core/src/math/vector4.rs` if the file has no existing `#[cfg(test)]` module; if one exists, add the tests inside it.

```rust
#[cfg(test)]
mod tests {
    use super::Vector4T;

    #[test]
    fn public_get_returns_vector4_components() {
        let v = Vector4T::new(10, 20, 30, 40);

        assert_eq!(v.get(0), 10);
        assert_eq!(v.get(1), 20);
        assert_eq!(v.get(2), 30);
        assert_eq!(v.get(3), 40);
    }

    #[test]
    fn public_set_updates_vector4_components() {
        let mut v = Vector4T::new(1, 2, 3, 4);

        v.set(0, 10);
        v.set(1, 20);
        v.set(2, 30);
        v.set(3, 40);

        assert_eq!(v, Vector4T::new(10, 20, 30, 40));
    }

    #[test]
    #[should_panic(expected = "Vector4 index out of range")]
    fn public_get_panics_for_out_of_range_vector4_index() {
        let v = Vector4T::new(1, 2, 3, 4);
        let _ = v.get(4);
    }

    #[test]
    #[should_panic(expected = "Vector4 index out of range")]
    fn public_set_panics_for_out_of_range_vector4_index() {
        let mut v = Vector4T::new(1, 2, 3, 4);
        v.set(usize::MAX, 5);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p voxel-core math::vector4::tests::public_get_panics_for_out_of_range_vector4_index --locked`

Expected: FAIL or abort from the existing unchecked fallback in release/test execution when invalid `get` reaches `unreachable_unchecked()`. If debug assertions trigger first, the failure still demonstrates the public method is not using the intended stable panic message.

- [ ] **Step 3: Write minimal implementation**

Replace `Vector4T::get` and `Vector4T::set` with:

```rust
    /// Index access with runtime bounds check. Mirrors `operator[]`.
    #[inline]
    pub fn get(&self, i: usize) -> T {
        match i {
            0 => self.x,
            1 => self.y,
            2 => self.z,
            3 => self.w,
            _ => panic!("Vector4 index out of range"),
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: T) {
        match i {
            0 => self.x = v,
            1 => self.y = v,
            2 => self.z = v,
            3 => self.w = v,
            _ => panic!("Vector4 index out of range"),
        }
    }
```

- [ ] **Step 4: Run focused vector4 tests**

Run: `cargo test -p voxel-core math::vector4::tests --locked`

Expected: PASS.

- [ ] **Step 5: Search for remaining unchecked fallback in vector4**

Run: `rg -n "unreachable_unchecked" rust/voxel-core/src/math/vector4.rs`

Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add rust/voxel-core/src/math/vector4.rs
git commit -m "fix(rust): make vector4 accessors panic safely"
```

### Task 3: Verify Safe API Coverage

**Files:**
- Modify: none after previous tasks.
- Test: workspace search and focused tests.

**Interfaces:**
- Consumes: completed vector3/vector4 changes.
- Produces: evidence that public vector accessors no longer expose reachable unchecked unsafe.

- [ ] **Step 1: Search math vectors for unchecked unsafe**

Run: `rg -n "unreachable_unchecked" rust/voxel-core/src/math`

Expected: no output for `vector3.rs` and `vector4.rs`. Any other math hit must be assessed before completion.

- [ ] **Step 2: Run all math vector tests**

Run: `cargo test -p voxel-core math::vector --locked`

Expected: PASS.

- [ ] **Step 3: Commit if Task 1 and Task 2 were not already committed**

```bash
git add rust/voxel-core/src/math/vector3.rs rust/voxel-core/src/math/vector4.rs
git commit -m "fix(rust): remove unsafe vector access fallbacks"
```
