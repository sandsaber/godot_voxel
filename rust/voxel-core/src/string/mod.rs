//! String utilities.
//!
//! Ported from `util/string/`. Submodules:
//! - [`conv`] — number↔bytes conversions (ported from `util/string/conv.{h,cpp}`).
//! - [`format`] — `{}`-placeholder formatting + hex-dump (from `format.{h,cpp}`).
//!
//! ## Skipped / deferred
//!
//! **Skipped** (C++ allocator plumbing with no Rust equivalent — Rust's `String`/
//! `&str` are the native types):
//! - `std_string.h` — `StdString` typedef over a custom STL allocator.
//! - `std_stringstream.h` — `StdStringStream` typedef (same).
//! - `fwd_std_string.h` — forward-declaration type tunneling (a C++ ABI trick).
//!
//! **Deferred to Phase 3** (`generators/graph` is the only consumer, not yet ported):
//! - `expression_parser.{h,cpp}` — a small recursive-descent math-expression
//!   parser producing an AST of `Number`/`Variable`/`Operator`/`Function` nodes.
//!   Used by the voxel graph compiler. ~640 LOC; revisit when `generators/graph`
//!   is ported so the `Function` callback table has real consumers.

pub mod conv;
pub mod format;

// Convenience re-exports.
pub use format::{format, to_hex_table};
