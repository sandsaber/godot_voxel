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
/// built on `std::mutex` + `std::condition_variable`). Used as the blocking
/// primitive inside [`SpatialLock3D`].
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
/// **Stub implementation.** Ported from `util/thread/spatial_lock_3d.{h,cpp}`
/// to preserve the C++ transcription surface — methods take the same
/// arguments and the [`SpatialLock3D::Read`] / [`SpatialLock3D::Write`]
/// guards exist so future port work on `VoxelData` can keep the per-method
/// guard variables 1:1 with C++. The guards here are **no-ops**: they
/// record nothing and provide no actual exclusion. Safety today comes from
/// `VoxelData` taking `&mut self` for every mutation (the borrow checker
/// enforces exclusivity at the type level).
///
/// When terrain worker threads land, replace this with a real
/// `Vec<Box<Mode>>` + [`Semaphore`] retry loop (see C++
/// `spatial_lock_3d.h:48-104`). The public API and guard types are designed
/// to remain stable across that swap.
#[derive(Debug, Default)]
pub struct SpatialLock3D {
    // Intentionally empty: the no-op stub provides no tracking. The field
    // exists so the type still has non-zero size and a stable layout when
    // the real implementation replaces it.
    _placeholder: (),
}

impl SpatialLock3D {
    pub fn new() -> Self {
        Self::default()
    }

    /// No-op: always succeeds. Real impl: returns `false` if an overlapping
    /// box is held in a conflicting mode.
    pub fn try_lock_read(&self, _bounds: crate::math::BoxBounds3i) -> bool {
        true
    }

    /// No-op: always succeeds immediately.
    pub fn lock_read(&self, _bounds: crate::math::BoxBounds3i) {}

    /// No-op.
    pub fn unlock_read(&self, _bounds: crate::math::BoxBounds3i) {}

    /// No-op: always succeeds.
    pub fn try_lock_write(&self, _bounds: crate::math::BoxBounds3i) -> bool {
        true
    }

    /// No-op: always succeeds immediately.
    pub fn lock_write(&self, _bounds: crate::math::BoxBounds3i) {}

    /// No-op.
    pub fn unlock_write(&self, _bounds: crate::math::BoxBounds3i) {}

    /// No-op stub returns 0.
    pub fn locked_boxes_count(&self) -> usize {
        0
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

/// RAII read guard for [`SpatialLock3D`]. Releases on drop. No-op in the
/// current stub; will call `unlock_read` once the lock is real.
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
    fn spatial_lock_3d_stub_provides_no_op_read_and_write_guards() {
        use super::SpatialLock3D;
        use crate::math::BoxBounds3i;
        let lock = SpatialLock3D::new();
        let bounds = BoxBounds3i::from_position(crate::math::Vector3i::new(1, 2, 3));
        // Locks always succeed in the stub.
        assert!(lock.try_lock_read(bounds));
        assert!(lock.try_lock_write(bounds));
        // Guards drop without panicking.
        {
            let _read = lock.read(bounds);
            let _write = lock.write(bounds);
        }
        assert_eq!(lock.locked_boxes_count(), 0);
    }
}
