//! Container algorithms ported from `util/containers/container_funcs.h`.
//!
//! Many of these are one-liners in std and are documented as such rather than
//! re-implemented; the ones with non-obvious semantics (`shift_up`,
//! `unordered_remove_*`, `find_duplicate`, `is_uniform`) are ported faithfully.

/// Take elements starting at `pos`, move them to the front, then truncate.
/// Other elements are discarded. Matches `shift_up`.
///
/// Idiomatic Rust can also express this as `v.drain(..pos);`.
pub fn shift_up<T>(v: &mut Vec<T>, pos: usize) {
    if pos == 0 || pos > v.len() {
        return;
    }
    v.drain(..pos);
}

/// Remove the element at `pos` by swapping it with the last, then popping.
/// Order is **not** preserved; O(1). Matches `unordered_remove`.
///
/// This is exactly [`Vec::swap_remove`] — prefer the native method.
#[inline]
pub fn unordered_remove<T>(v: &mut Vec<T>, pos: usize) {
    v.swap_remove(pos);
}

/// Remove every element for which `predicate` is true. Order is **not**
/// preserved; O(n). Matches `unordered_remove_if`. (For order-preserving
/// removal use [`Vec::retain`] with the negated predicate.)
pub fn unordered_remove_if<T, F: FnMut(&T) -> bool>(v: &mut Vec<T>, mut predicate: F) {
    let mut i = 0;
    while i < v.len() {
        if predicate(&v[i]) {
            v.swap_remove(i);
        } else {
            i += 1;
        }
    }
}

/// Remove the first element equal to `value` (swap-remove). Returns true if an
/// element was removed. Matches `unordered_remove_value`.
pub fn unordered_remove_value<T: PartialEq>(v: &mut Vec<T>, value: &T) -> bool {
    if let Some(i) = v.iter().position(|x| x == value) {
        v.swap_remove(i);
        true
    } else {
        false
    }
}

/// Append all of `src` to the end of `dst`. Matches `append_array` — this is
/// exactly [`Vec::extend_from_slice`].
#[inline]
pub fn append_array<T: Clone>(dst: &mut Vec<T>, src: &[T]) {
    dst.extend_from_slice(src);
}

/// First pair of equal indices in `items`, or `None` if all are distinct.
/// Matches `find_duplicate`. O(n²) — same as the C++ implementation; for large
/// inputs prefer a `HashSet`.
pub fn find_duplicate<T: PartialEq>(items: &[T]) -> Option<(usize, usize)> {
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            if items[i] == items[j] {
                return Some((i, j));
            }
        }
    }
    None
}

/// Like [`find_duplicate`] but with a custom equality function. Matches
/// `find_duplicate_f`.
pub fn find_duplicate_with<T, F: Fn(&T, &T) -> bool>(
    items: &[T],
    equal: F,
) -> Option<(usize, usize)> {
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            if equal(&items[i], &items[j]) {
                return Some((i, j));
            }
        }
    }
    None
}

/// True if any two elements are equal. Matches `has_duplicate`.
#[inline]
pub fn has_duplicate<T: PartialEq>(items: &[T]) -> bool {
    find_duplicate(items).is_some()
}

/// True if every element equals the first (i.e. the buffer is uniform). An
/// empty slice is not uniform. Matches `is_uniform` (without the C++ bucket
/// micro-optimization, which LLVM vectorizes the simple loop to match).
pub fn is_uniform<T: PartialEq>(items: &[T]) -> bool {
    match items.first() {
        Some(first) => items.iter().all(|x| x == first),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_up_drops_prefix() {
        let mut v = vec![1, 2, 3, 4, 5];
        shift_up(&mut v, 2);
        assert_eq!(v, vec![3, 4, 5]);
        let mut w = vec![1, 2, 3];
        shift_up(&mut w, 0);
        assert_eq!(w, vec![1, 2, 3]);
        let mut u = vec![1, 2];
        shift_up(&mut u, 5); // out of range: no-op
        assert_eq!(u, vec![1, 2]);
        let mut clear = vec![1, 2];
        shift_up(&mut clear, 2);
        assert_eq!(clear, Vec::<i32>::new());
    }

    #[test]
    fn unordered_remove_if_preserves_set_not_order() {
        let mut v = vec![1, 2, 3, 4, 5];
        unordered_remove_if(&mut v, |x| *x % 2 == 0);
        assert_eq!(v.len(), 3);
        assert!(v.iter().all(|x| x % 2 == 1));
    }

    #[test]
    fn unordered_remove_value_returns_and_removes() {
        let mut v = vec![10, 20, 30];
        assert!(unordered_remove_value(&mut v, &20));
        assert_eq!(v.len(), 2);
        assert!(!v.contains(&20));
        assert!(!unordered_remove_value(&mut v, &99));
    }

    #[test]
    fn duplicate_detection() {
        assert_eq!(find_duplicate(&[1, 2, 3, 2]), Some((1, 3)));
        assert_eq!(find_duplicate(&[1, 2, 3]), None);
        assert!(has_duplicate(&[1, 1, 2]));
        assert!(!has_duplicate(&[1, 2, 3]));
    }

    #[test]
    fn is_uniform_check() {
        assert!(is_uniform(&[7u32; 5]));
        assert!(!is_uniform(&[7u32, 7, 8]));
        assert!(!is_uniform::<u32>(&[]));
    }

    #[test]
    fn append_array_works() {
        let mut a = vec![1, 2];
        append_array(&mut a, &[3, 4]);
        assert_eq!(a, vec![1, 2, 3, 4]);
    }
}
