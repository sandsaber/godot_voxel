//! Memory allocation.
//!
//! Ported from `util/memory/{memory,std_allocator}.h`. In C++ these select
//! between Godot's allocator and the default one (`ZN_NEW`/`ZN_DELETE`/
//! `ZN_ALLOC`/`ZN_REALLOC`/`ZN_FREE`) and provide a debug allocation counter
//! (`StdDefaultAllocatorCounters`).
//!
//! ## Rust mapping
//!
//! Rust uses a single *global allocator* (customizable via `#[global_allocator]`)
//! and owns heap memory through `Box<T>` / `Vec<T>` / `Rc<T>` / `Arc<T>`, which
//! automatically deallocate on drop. There is therefore no need for the C++
//! `ZN_ALLOC`/`ZN_FREE` macros or the `StdDefaultAllocator<T>` STL wrapper —
//! `voxel-core` just uses `Vec`/`Box` directly, and the `voxel-gdext` crate can
//! install Godot's allocator globally when it links against the engine.
//!
//! | C++ | Rust |
//! |-----|------|
//! | `ZN_NEW(T(...))` / `ZN_DELETE(p)` | `Box::new(T(...))` / drop the `Box<T>` |
//! | `UniquePtr<T>` | `Box<T>` |
//! | `make_unique_instance<T>(...)` | `Box::new(T { ... })` |
//! | `make_shared_instance<T>(...)` | `Rc::new(...)` or `Arc::new(...)` |
//! | `ZN_ALLOC(n)` / `ZN_FREE(p)` | `Vec::<u8>::with_capacity(n)` / drop |
//! | `ZN_REALLOC(p, n)` | `Vec::resize` / `Vec` amortized growth |
//! | `StdDefaultAllocator<T>` | (implicit — Rust's global allocator) |
//!
//! ## Debug allocation counters
//!
//! The one piece worth porting is the debug counters (`g_allocated` /
//! `g_deallocated`), used to spot leaks in long-running operations. Exposed here
//! behind the `alloc-counters` feature; off by default to avoid the atomic
//! overhead in release paths.

#![cfg_attr(not(feature = "alloc-counters"), allow(unused))]

#[cfg(feature = "alloc-counters")]
mod counters {
    use core::sync::atomic::{AtomicU64, Ordering};

    /// Total bytes ever allocated (monotonic). Mirrors `g_allocated`.
    pub static ALLOCATED: AtomicU64 = AtomicU64::new(0);

    /// Total bytes ever deallocated (monotonic). Mirrors `g_deallocated`.
    pub static DEALLOCATED: AtomicU64 = AtomicU64::new(0);

    /// Record an allocation of `bytes` bytes. Call from a custom allocator's
    /// `alloc` if you want parity with the C++ counters.
    #[inline]
    pub fn track_alloc(bytes: u64) {
        ALLOCATED.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record a deallocation of `bytes` bytes.
    #[inline]
    pub fn track_dealloc(bytes: u64) {
        DEALLOCATED.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Bytes currently live (`allocated − deallocated`). Negative underflow wraps
    /// to a huge value, which itself signals a bookkeeping bug.
    #[inline]
    pub fn current_usage() -> i64 {
        let a = ALLOCATED.load(Ordering::Relaxed) as i64;
        let d = DEALLOCATED.load(Ordering::Relaxed) as i64;
        a - d
    }

    /// Reset both counters to zero (test helper).
    #[inline]
    pub fn reset() {
        ALLOCATED.store(0, Ordering::Relaxed);
        DEALLOCATED.store(0, Ordering::Relaxed);
    }
}

#[cfg(feature = "alloc-counters")]
pub use counters::*;

#[cfg(test)]
#[cfg(feature = "alloc-counters")]
mod tests {
    use super::*;

    #[test]
    fn counters_track_balance() {
        reset();
        track_alloc(100);
        track_alloc(50);
        assert_eq!(current_usage(), 150);
        track_dealloc(80);
        assert_eq!(current_usage(), 70);
    }

    #[test]
    fn counters_are_monotonic() {
        reset();
        track_alloc(10);
        track_alloc(20);
        // ALLOCATED only ever grows.
        assert!(ALLOCATED.load(core::sync::atomic::Ordering::Relaxed) >= 30);
    }
}
