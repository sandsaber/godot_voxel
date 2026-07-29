//! Fixed-size arrays — the Rust analogue of C++ `FixedArray<T, N>`.
//!
//! `FixedArray<T, N>` maps directly to `[T; N]`. Indexing (bounds-checked),
//! `len()`, iteration and `fill` are all native. The free helpers from
//! `fixed_array.h` map to slice methods (available on arrays by deref):
//!
//! | C++ free function                | Rust                          |
//! |----------------------------------|-------------------------------|
//! | `fill(a, v)`                     | `a.fill(v)`                   |
//! | `contains(a, v)`                 | [`contains`](fn.contains.html)|
//! | `find(a, v, out_index)`          | [`find`](fn.find.html)        |
//! | `to_span(a)` / `to_span_const`   | `&a[..]` / `&mut a[..]`       |

/// True if `value` is present in `items`. Matches `fixed_array::contains` /
/// `container_funcs::contains(Span)`.
pub fn contains<T: PartialEq>(items: &[T], value: &T) -> bool {
    items.iter().any(|x| x == value)
}

/// Index of the first element equal to `value`, or `None`. Matches
/// `fixed_array::find` / `container_funcs::find(Span, value)`.
pub fn find<T: PartialEq>(items: &[T], value: &T) -> Option<usize> {
    items.iter().position(|x| x == value)
}

/// Index of the first element matching `predicate`, or `None`. Matches
/// `container_funcs::find(Span, predicate)`.
pub fn find_with<T, F: FnMut(&T) -> bool>(items: &[T], predicate: F) -> Option<usize> {
    items.iter().position(predicate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_and_find() {
        let a = [1, 2, 3, 4];
        assert!(contains(&a, &3));
        assert!(!contains(&a, &9));
        assert_eq!(find(&a, &3), Some(2));
        assert_eq!(find(&a, &9), None);
        assert_eq!(find_with(&a, |x| *x > 2), Some(2));
    }
}
