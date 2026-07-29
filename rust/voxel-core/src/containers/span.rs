//! Slice views — the Rust analogue of C++ `Span<T>`.
//!
//! `Span<T>` and `Span<const T>` map directly to Rust slices:
//! - `Span<T>`       → `&mut [T]`
//! - `Span<const T>` → `&[T]`
//!
//! Most `Span` operations are native slice methods: bounds-checked indexing
//! (`s[i]`), `len`, `is_empty`, iteration, sub-views (`&s[from..end]`),
//! `fill`, `copy_from_slice` (= `Span::copy_to`). A single element view uses
//! [`core::slice::from_ref`] / [`core::slice::from_mut`] (= `to_single_element_span`).

/// Do the two slices reference overlapping byte ranges?
///
/// Matches `Span::overlaps` (pointer-range comparison). Only meaningful for
/// sub-slices of the same allocation; the borrow checker already prevents
/// overlapping `&mut` borrows, so this is rarely needed in safe Rust — it's
/// provided for API parity and for `&[T]` aliasing checks.
pub fn overlaps<T>(a: &[T], b: &[T]) -> bool {
    let ap = a.as_ptr() as usize;
    let bp = b.as_ptr() as usize;
    ap + core::mem::size_of_val(a) > bp && ap < bp + core::mem::size_of_val(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlaps_within_one_buffer() {
        let buf = [0u8; 16];
        let a = &buf[0..8];
        let b = &buf[4..12];
        assert!(overlaps(a, b));
        let c = &buf[12..16];
        assert!(!overlaps(a, c));
    }
}
