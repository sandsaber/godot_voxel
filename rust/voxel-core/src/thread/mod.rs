//! Threading primitives ported from `util/thread/`.
//!
//! The C++ layer exposes locks as standalone synchronization objects:
//! `Mutex` is recursive (`std::recursive_mutex`), `BinaryMutex` is non-recursive
//! (`std::mutex`), and `RWLock` wraps `std::shared_timed_mutex`. These Rust
//! wrappers keep the same lock-object shape and return RAII guards.

use std::sync::{
    Condvar, Mutex as StdMutex, MutexGuard as StdMutexGuard, RwLock as StdRwLock,
    RwLockReadGuard as StdRwLockReadGuard, RwLockWriteGuard as StdRwLockWriteGuard, TryLockError,
};
use std::thread::ThreadId;

#[derive(Debug, Default)]
struct RecursiveState {
    owner: Option<ThreadId>,
    depth: usize,
}

/// Recursive mutex. Ported from C++ `Mutex` (`std::recursive_mutex`).
#[derive(Debug, Default)]
pub struct Mutex {
    state: StdMutex<RecursiveState>,
    cvar: Condvar,
}

impl Mutex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock recursively on the current thread, blocking until available.
    pub fn lock(&self) -> MutexGuard<'_> {
        let current = std::thread::current().id();
        let mut state = lock_unpoisoned(&self.state);
        loop {
            match state.owner {
                None => {
                    state.owner = Some(current);
                    state.depth = 1;
                    return MutexGuard::new(self);
                }
                Some(owner) if owner == current => {
                    state.depth += 1;
                    return MutexGuard::new(self);
                }
                Some(_) => {
                    state = wait_unpoisoned(&self.cvar, state);
                }
            }
        }
    }

    /// Try to lock recursively on the current thread.
    pub fn try_lock(&self) -> Option<MutexGuard<'_>> {
        let current = std::thread::current().id();
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        match state.owner {
            None => {
                state.owner = Some(current);
                state.depth = 1;
                Some(MutexGuard::new(self))
            }
            Some(owner) if owner == current => {
                state.depth += 1;
                Some(MutexGuard::new(self))
            }
            Some(_) => None,
        }
    }

    fn unlock(&self) {
        let current = std::thread::current().id();
        let mut state = lock_unpoisoned(&self.state);
        debug_assert_eq!(state.owner, Some(current));
        debug_assert!(state.depth > 0);
        state.depth -= 1;
        if state.depth == 0 {
            state.owner = None;
            self.cvar.notify_one();
        }
    }
}

/// RAII guard returned by [`Mutex::lock`] / [`Mutex::try_lock`].
#[derive(Debug)]
pub struct MutexGuard<'a> {
    lock: &'a Mutex,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl<'a> MutexGuard<'a> {
    fn new(lock: &'a Mutex) -> Self {
        Self {
            lock,
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for MutexGuard<'_> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}

/// Non-recursive mutex. Ported from C++ `BinaryMutex` (`std::mutex`).
#[derive(Debug, Default)]
pub struct BinaryMutex {
    inner: StdMutex<()>,
}

impl BinaryMutex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lock(&self) -> BinaryMutexGuard<'_> {
        BinaryMutexGuard {
            _guard: lock_unpoisoned(&self.inner),
        }
    }

    pub fn try_lock(&self) -> Option<BinaryMutexGuard<'_>> {
        match self.inner.try_lock() {
            Ok(guard) => Some(BinaryMutexGuard { _guard: guard }),
            Err(TryLockError::Poisoned(e)) => Some(BinaryMutexGuard {
                _guard: e.into_inner(),
            }),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

/// RAII guard returned by [`BinaryMutex`].
#[derive(Debug)]
pub struct BinaryMutexGuard<'a> {
    _guard: StdMutexGuard<'a, ()>,
}

/// Read/write lock. Ported from C++ `RWLock` (`std::shared_timed_mutex`).
#[derive(Debug, Default)]
pub struct RwLock {
    inner: StdRwLock<()>,
}

impl RwLock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_lock(&self) -> RwLockReadGuard<'_> {
        RwLockReadGuard {
            _guard: read_unpoisoned(&self.inner),
        }
    }

    pub fn read_try_lock(&self) -> Option<RwLockReadGuard<'_>> {
        match self.inner.try_read() {
            Ok(guard) => Some(RwLockReadGuard { _guard: guard }),
            Err(TryLockError::Poisoned(e)) => Some(RwLockReadGuard {
                _guard: e.into_inner(),
            }),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    pub fn write_lock(&self) -> RwLockWriteGuard<'_> {
        RwLockWriteGuard {
            _guard: write_unpoisoned(&self.inner),
        }
    }

    pub fn write_try_lock(&self) -> Option<RwLockWriteGuard<'_>> {
        match self.inner.try_write() {
            Ok(guard) => Some(RwLockWriteGuard { _guard: guard }),
            Err(TryLockError::Poisoned(e)) => Some(RwLockWriteGuard {
                _guard: e.into_inner(),
            }),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

/// RAII read guard returned by [`RwLock`].
#[derive(Debug)]
pub struct RwLockReadGuard<'a> {
    _guard: StdRwLockReadGuard<'a, ()>,
}

/// RAII write guard returned by [`RwLock`].
#[derive(Debug)]
pub struct RwLockWriteGuard<'a> {
    _guard: StdRwLockWriteGuard<'a, ()>,
}

fn lock_unpoisoned<T>(mutex: &StdMutex<T>) -> StdMutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn wait_unpoisoned<'a, T>(cvar: &Condvar, guard: StdMutexGuard<'a, T>) -> StdMutexGuard<'a, T> {
    cvar.wait(guard).unwrap_or_else(|e| e.into_inner())
}

fn read_unpoisoned<T>(lock: &StdRwLock<T>) -> StdRwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| e.into_inner())
}

fn write_unpoisoned<T>(lock: &StdRwLock<T>) -> StdRwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|e| e.into_inner())
}

/// Counting semaphore. Ported from `util/thread/semaphore.h` (header-only,
/// built on `std::mutex` + `std::condition_variable`).
///
/// Hand-rolled with `Mutex<usize>` + `Condvar` to keep the crate dependency-
/// free (the stdlib `Semaphore` is unstable; `parking_lot::Semaphore` would
/// add a runtime dep).
#[derive(Debug, Default)]
pub struct Semaphore {
    state: StdMutex<usize>,
    cvar: Condvar,
}

impl Semaphore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_count(count: usize) -> Self {
        Self {
            state: StdMutex::new(count),
            cvar: Condvar::new(),
        }
    }

    /// Increment the counter and wake one waiter.
    pub fn post(&self) {
        let mut count = lock_unpoisoned(&self.state);
        *count = count.saturating_add(1);
        self.cvar.notify_one();
    }

    /// Block until the counter is non-zero, then decrement it.
    pub fn wait(&self) {
        let mut count = lock_unpoisoned(&self.state);
        while *count == 0 {
            count = wait_unpoisoned(&self.cvar, count);
        }
        *count -= 1;
    }

    /// Decrement the counter if non-zero; returns `true` on success.
    pub fn try_wait(&self) -> bool {
        let mut count = lock_unpoisoned(&self.state);
        if *count == 0 {
            return false;
        }
        *count -= 1;
        true
    }

    pub fn count(&self) -> usize {
        *lock_unpoisoned(&self.state)
    }
}

/// Mode of a [`SpatialLock3D`] area guard. Mirrors C++ `SpatialLock3D::Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialLockMode {
    Read,
    Write,
}

/// Region-based read/write lock over 3D integer boxes.
///
/// Ported from `util/thread/spatial_lock_3d.{h,cpp}`. Multiple read locks may
/// overlap; a write lock excludes every overlapping read or write lock.
/// Disjoint boxes can proceed concurrently.
#[derive(Debug, Default)]
pub struct SpatialLock3D {
    state: StdMutex<Vec<SpatialLockEntry>>,
    cvar: Condvar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpatialLockEntry {
    bounds: crate::math::BoxBounds3i,
    mode: SpatialLockMode,
}

impl SpatialLock3D {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `false` if an overlapping box is currently held for writing.
    pub fn try_lock_read(&self, bounds: crate::math::BoxBounds3i) -> bool {
        let mut entries = lock_unpoisoned(&self.state);
        if spatial_lock_can_acquire(&entries, bounds, SpatialLockMode::Read) {
            entries.push(SpatialLockEntry {
                bounds,
                mode: SpatialLockMode::Read,
            });
            true
        } else {
            false
        }
    }

    /// Block until no overlapping write lock exists.
    pub fn lock_read(&self, bounds: crate::math::BoxBounds3i) {
        let mut entries = lock_unpoisoned(&self.state);
        while !spatial_lock_can_acquire(&entries, bounds, SpatialLockMode::Read) {
            entries = wait_unpoisoned(&self.cvar, entries);
        }
        entries.push(SpatialLockEntry {
            bounds,
            mode: SpatialLockMode::Read,
        });
    }

    pub fn unlock_read(&self, bounds: crate::math::BoxBounds3i) {
        self.unlock(bounds, SpatialLockMode::Read);
    }

    /// Returns `false` if any overlapping read or write lock exists.
    pub fn try_lock_write(&self, bounds: crate::math::BoxBounds3i) -> bool {
        let mut entries = lock_unpoisoned(&self.state);
        if spatial_lock_can_acquire(&entries, bounds, SpatialLockMode::Write) {
            entries.push(SpatialLockEntry {
                bounds,
                mode: SpatialLockMode::Write,
            });
            true
        } else {
            false
        }
    }

    /// Block until no overlapping lock exists.
    pub fn lock_write(&self, bounds: crate::math::BoxBounds3i) {
        let mut entries = lock_unpoisoned(&self.state);
        while !spatial_lock_can_acquire(&entries, bounds, SpatialLockMode::Write) {
            entries = wait_unpoisoned(&self.cvar, entries);
        }
        entries.push(SpatialLockEntry {
            bounds,
            mode: SpatialLockMode::Write,
        });
    }

    pub fn unlock_write(&self, bounds: crate::math::BoxBounds3i) {
        self.unlock(bounds, SpatialLockMode::Write);
    }

    pub fn locked_boxes_count(&self) -> usize {
        lock_unpoisoned(&self.state).len()
    }

    fn unlock(&self, bounds: crate::math::BoxBounds3i, mode: SpatialLockMode) {
        let mut entries = lock_unpoisoned(&self.state);
        let Some(index) = entries
            .iter()
            .position(|entry| entry.bounds == bounds && entry.mode == mode)
        else {
            debug_assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry.bounds == bounds && entry.mode == mode)
                    .count(),
                1,
                "unlock called for a SpatialLock3D entry that is not held"
            );
            return;
        };
        entries.swap_remove(index);
        self.cvar.notify_all();
    }

    /// Convenience: acquire a read lock for `bounds` and return an RAII guard.
    /// Mirrors the C++ `SpatialLock3D::Read` nested type.
    pub fn read(&self, bounds: crate::math::BoxBounds3i) -> SpatialLockReadGuard<'_> {
        self.lock_read(bounds);
        SpatialLockReadGuard { lock: self, bounds }
    }

    /// Convenience: acquire a write lock for `bounds` and return an RAII guard.
    /// Mirrors the C++ `SpatialLock3D::Write` nested type.
    pub fn write(&self, bounds: crate::math::BoxBounds3i) -> SpatialLockWriteGuard<'_> {
        self.lock_write(bounds);
        SpatialLockWriteGuard { lock: self, bounds }
    }
}

/// RAII read guard for [`SpatialLock3D`]. Releases on drop.
#[derive(Debug)]
pub struct SpatialLockReadGuard<'a> {
    lock: &'a SpatialLock3D,
    bounds: crate::math::BoxBounds3i,
}

impl Drop for SpatialLockReadGuard<'_> {
    fn drop(&mut self) {
        self.lock.unlock_read(self.bounds);
    }
}

/// RAII write guard for [`SpatialLock3D`]. Releases on drop.
#[derive(Debug)]
pub struct SpatialLockWriteGuard<'a> {
    lock: &'a SpatialLock3D,
    bounds: crate::math::BoxBounds3i,
}

impl Drop for SpatialLockWriteGuard<'_> {
    fn drop(&mut self) {
        self.lock.unlock_write(self.bounds);
    }
}

fn spatial_lock_can_acquire(
    entries: &[SpatialLockEntry],
    bounds: crate::math::BoxBounds3i,
    mode: SpatialLockMode,
) -> bool {
    entries.iter().all(|entry| {
        if !entry.bounds.intersects(&bounds) {
            return true;
        }
        matches!(
            (mode, entry.mode),
            (SpatialLockMode::Read, SpatialLockMode::Read)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{BinaryMutex, Mutex, RwLock};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn mutex_allows_recursive_locking_on_same_thread() {
        let lock = Mutex::new();
        let _outer = lock.lock();
        assert!(lock.try_lock().is_some());
    }

    #[test]
    fn binary_mutex_try_lock_fails_while_held() {
        let lock = BinaryMutex::new();
        let _guard = lock.lock();
        assert!(lock.try_lock().is_none());
    }

    #[test]
    fn rw_lock_allows_multiple_readers_but_excludes_writer() {
        let lock = Arc::new(RwLock::new());
        let read_a = lock.read_lock();
        let read_b = lock.read_lock();
        assert!(lock.write_try_lock().is_none());
        drop(read_a);
        assert!(lock.write_try_lock().is_none());
        drop(read_b);
        assert!(lock.write_try_lock().is_some());
    }

    #[test]
    fn rw_lock_writer_excludes_readers_across_threads() {
        let lock = Arc::new(RwLock::new());
        let writer = lock.write_lock();
        let (tx, rx) = mpsc::channel();
        let worker_lock = lock.clone();

        let handle = std::thread::spawn(move || {
            tx.send(worker_lock.read_try_lock().is_none()).unwrap();
        });

        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap());
        drop(writer);
        handle.join().unwrap();
        assert!(lock.read_try_lock().is_some());
    }

    #[test]
    fn semaphore_try_wait_returns_false_at_zero_and_true_after_post() {
        use super::Semaphore;
        let sem = Semaphore::new();
        assert_eq!(sem.count(), 0);
        assert!(!sem.try_wait());
        sem.post();
        sem.post();
        assert_eq!(sem.count(), 2);
        assert!(sem.try_wait());
        assert_eq!(sem.count(), 1);
    }

    #[test]
    fn semaphore_wait_blocks_until_another_thread_posts() {
        use super::Semaphore;
        let sem = Arc::new(Semaphore::new());
        let worker_sem = sem.clone();
        let handle = std::thread::spawn(move || {
            // Worker waits (will block until the main thread posts).
            worker_sem.wait();
        });
        // Give the worker a moment to enter wait(), then post.
        std::thread::sleep(Duration::from_millis(20));
        sem.post();
        handle.join().expect("worker should unblock after post");
    }

    #[test]
    fn spatial_lock_3d_respects_overlap_and_mode() {
        use super::SpatialLock3D;
        use crate::math::{BoxBounds3i, Vector3i};
        let lock = SpatialLock3D::new();
        let area = BoxBounds3i::new(Vector3i::zero(), Vector3i::new(4, 4, 4));
        let overlap = BoxBounds3i::new(Vector3i::new(2, 2, 2), Vector3i::new(6, 6, 6));
        let disjoint = BoxBounds3i::new(Vector3i::new(8, 8, 8), Vector3i::new(10, 10, 10));

        let read = lock.read(area);
        assert!(lock.try_lock_read(overlap), "overlapping reads may coexist");
        assert_eq!(lock.locked_boxes_count(), 2);
        assert!(
            !lock.try_lock_write(overlap),
            "overlapping write must wait for readers"
        );
        assert!(
            lock.try_lock_write(disjoint),
            "disjoint write can run alongside reads"
        );
        assert_eq!(lock.locked_boxes_count(), 3);
        lock.unlock_write(disjoint);
        lock.unlock_read(overlap);
        drop(read);

        assert!(lock.try_lock_write(overlap));
        assert_eq!(lock.locked_boxes_count(), 1);
        lock.unlock_write(overlap);
        assert_eq!(lock.locked_boxes_count(), 0);
    }

    #[test]
    fn spatial_lock_3d_blocking_write_waits_for_overlapping_read() {
        use super::SpatialLock3D;
        use crate::math::{BoxBounds3i, Vector3i};

        let lock = Arc::new(SpatialLock3D::new());
        let bounds = BoxBounds3i::new(Vector3i::zero(), Vector3i::new(4, 4, 4));
        let read = lock.read(bounds);
        let worker_lock = lock.clone();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            attempt_tx.send(()).unwrap();
            let _write = worker_lock.write(bounds);
            acquired_tx.send(()).unwrap();
        });

        attempt_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            acquired_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "writer acquired overlapping region before read guard was dropped"
        );
        drop(read);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }
}
