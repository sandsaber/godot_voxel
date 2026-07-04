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
}
