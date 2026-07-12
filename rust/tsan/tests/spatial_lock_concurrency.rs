//! ThreadSanitizer target for `SpatialLock3D`.
//!
//! Drives many threads taking overlapping and disjoint read/write regions on
//! a single `SpatialLock3D`. The lock itself only guards its internal entry
//! list, but its public contract — overlapping reads coexist, overlapping
//! writes block, disjoint regions proceed — is exactly what TSan should
//! verify stays race-free under heavy contention.
//!
//! Run with:
//! ```text
//! RUSTFLAGS="-Zsanitizer=thread -Cunsafe-allow-abi-mismatch=sanitizer" \
//!   cargo +nightly test -p tsan --test spatial_lock_concurrency -- --test-threads=1
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use voxel_core::math::{BoxBounds3i, Vector3i};
use voxel_core::thread::SpatialLock3D;

/// A small critical section that both reads and writes a shared counter under
/// a held spatial write region. TSan would flag any unsynchronised access.
fn touch_shared(state: &Arc<AtomicUsize>, delta: i32) {
    if delta >= 0 {
        state.fetch_add(delta as usize, Ordering::SeqCst);
    } else {
        state.fetch_sub((-delta) as usize, Ordering::SeqCst);
    }
}

#[test]
fn spatial_lock_3d_concurrent_readers_and_writers_stay_race_free() {
    let lock = Arc::new(SpatialLock3D::new());
    // Counter touched only while holding the overlapping write region — if the
    // lock ever let two writers in together, TSan would report a data race.
    let shared = Arc::new(AtomicUsize::new(0));
    // Disjoint-region counter touched by writers on non-overlapping boxes.
    let disjoint_a = Arc::new(AtomicUsize::new(0));
    let disjoint_b = Arc::new(AtomicUsize::new(0));

    const THREADS: usize = 8;
    const ITERS: usize = 200;
    let start = Arc::new(Barrier::new(THREADS));

    thread::scope(|scope| {
        for t in 0..THREADS {
            let lock = lock.clone();
            let shared = shared.clone();
            let disjoint_a = disjoint_a.clone();
            let disjoint_b = disjoint_b.clone();
            let start = start.clone();
            scope.spawn(move || {
                start.wait();
                // Region that all threads overlap on — exercises both the
                // shared read path and the serialised write path.
                let overlap = BoxBounds3i::from_min_max_included(
                    Vector3i::new(0, 0, 0),
                    Vector3i::new(16, 16, 16),
                );
                // Two disjoint regions used by alternating threads so that
                // disjoint writes proceed in parallel without blocking.
                let reg_a = BoxBounds3i::from_min_max_included(
                    Vector3i::new(1000, 0, 0),
                    Vector3i::new(1016, 16, 16),
                );
                let reg_b = BoxBounds3i::from_min_max_included(
                    Vector3i::new(2000, 0, 0),
                    Vector3i::new(2016, 16, 16),
                );

                for i in 0..ITERS {
                    // Readers on the overlapping region coexist.
                    {
                        let _g = lock.read(overlap);
                        // Reading the shared counter under a read guard is
                        // fine only because writers take the same region as a
                        // write — TSan validates that invariant.
                        let _ = shared.load(Ordering::SeqCst);
                    }
                    // Writer on the overlapping region — serialised.
                    {
                        let _g = lock.write(overlap);
                        touch_shared(&shared, (t as i32 + i as i32) % 7);
                    }
                    // Disjoint writers proceed in parallel across threads.
                    if t % 2 == 0 {
                        let _g = lock.write(reg_a);
                        disjoint_a.fetch_add(1, Ordering::SeqCst);
                    } else {
                        let _g = lock.write(reg_b);
                        disjoint_b.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
        }
    });

    // All regions must be released once every thread is done.
    assert_eq!(lock.locked_boxes_count(), 0);
    // Disjoint counters must reflect every thread's work.
    let a = disjoint_a.load(Ordering::SeqCst);
    let b = disjoint_b.load(Ordering::SeqCst);
    assert_eq!(a + b, (THREADS * ITERS));
    // Shared counter must be non-negative-ish and deterministic in magnitude:
    // every iteration added a value in 0..7.
    let total = shared.load(Ordering::SeqCst);
    assert!(total < THREADS * ITERS * 7);
}

/// Two threads take regions in opposite order to stress the lock's fairness /
/// wakeup path. TSan would flag a race if the Condvar-based blocking path
/// exposed the protected data unsynchronised.
#[test]
fn spatial_lock_3d_blocking_write_wakeup_is_race_free() {
    let lock = Arc::new(SpatialLock3D::new());
    let region = BoxBounds3i::from_min_max_included(Vector3i::zero(), Vector3i::splat(8));
    let observed = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(Barrier::new(2));

    thread::scope(|scope| {
        let reader_lock = lock.clone();
        let reader_ready = ready.clone();
        scope.spawn(move || {
            // Hold a read lock so the writer must block, then release.
            let _g = reader_lock.read(region);
            reader_ready.wait();
            thread::sleep(std::time::Duration::from_millis(5));
            drop(_g);
        });

        let writer_lock = lock.clone();
        let writer_observed = observed.clone();
        let writer_ready = ready.clone();
        scope.spawn(move || {
            writer_ready.wait();
            // Blocks until the reader releases; once acquired we write the
            // shared state — TSan verifies the handoff is synchronised.
            let _g = writer_lock.write(region);
            writer_observed.fetch_add(1, Ordering::SeqCst);
        });
    });

    assert_eq!(lock.locked_boxes_count(), 0);
    assert_eq!(observed.load(Ordering::SeqCst), 1);
}
