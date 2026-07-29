//! Container helpers ported from `util/containers/`.
//!
//! Rust already provides the underlying types natively, so this module is much
//! thinner than its C++ counterpart:
//!
//! | C++ (`util/containers/`)        | Rust                          |
//! |---------------------------------|-------------------------------|
//! | `Span<T>` (mutable view)        | `&mut [T]`                    |
//! | `Span<const T>` (read view)     | `&[T]`                        |
//! | `FixedArray<T, N>`              | `[T; N]`                      |
//! | `StdVector<T>` / `SmallVector`  | `Vec<T>`                      |
//!
//! [`funcs`] ports the algorithms from `container_funcs.h` that are not already
//! one-liners in std; [`span`] / [`fixed_array`] provide the few helpers the
//! native types lack as methods plus the API-parity free functions used by
//! downstream ports.

pub mod fixed_array;
pub mod funcs;
pub mod span;
