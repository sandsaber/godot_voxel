//! Software per-path read/write locks.
//!
//! Ported from `util/io/file_locker.h`. The C++ API locks by path and later
//! unlocks by path; Rust returns an owned RAII guard so the path is released
//! deterministically on drop and mixed read/write unlock state cannot drift.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};

/// Coordinates read/write access to logical file paths.
#[derive(Debug, Default)]
pub struct FileLocker {
    files: StdMutex<HashMap<PathBuf, Arc<PathLock>>>,
}

impl FileLocker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock a path for reading. Multiple readers for the same path may overlap.
    pub fn lock_read<P: AsRef<Path>>(&self, path: P) -> FileReadGuard {
        self.path_lock(path.as_ref()).lock_read()
    }

    /// Lock a path for writing. Excludes all readers and writers for the same
    /// path, while unrelated paths remain independent.
    pub fn lock_write<P: AsRef<Path>>(&self, path: P) -> FileWriteGuard {
        self.path_lock(path.as_ref()).lock_write()
    }

    /// Number of paths tracked by this locker. Mainly useful for invariants in
    /// tests; entries intentionally persist like the C++ map.
    pub fn tracked_path_count(&self) -> usize {
        lock_unpoisoned(&self.files).len()
    }

    fn path_lock(&self, path: &Path) -> Arc<PathLock> {
        let mut files = lock_unpoisoned(&self.files);
        files.entry(path.to_path_buf()).or_default().clone()
    }
}

#[derive(Debug, Default)]
struct PathLock {
    state: StdMutex<PathLockState>,
    cvar: Condvar,
}

impl PathLock {
    fn lock_read(self: Arc<Self>) -> FileReadGuard {
        let mut state = lock_unpoisoned(&self.state);
        while state.writer {
            state = wait_unpoisoned(&self.cvar, state);
        }
        state.readers += 1;
        drop(state);
        FileReadGuard { lock: self }
    }

    fn lock_write(self: Arc<Self>) -> FileWriteGuard {
        let mut state = lock_unpoisoned(&self.state);
        while state.writer || state.readers != 0 {
            state = wait_unpoisoned(&self.cvar, state);
        }
        state.writer = true;
        drop(state);
        FileWriteGuard { lock: self }
    }

    fn unlock_read(&self) {
        let mut state = lock_unpoisoned(&self.state);
        debug_assert!(state.readers > 0);
        state.readers -= 1;
        if state.readers == 0 {
            self.cvar.notify_all();
        }
    }

    fn unlock_write(&self) {
        let mut state = lock_unpoisoned(&self.state);
        debug_assert!(state.writer);
        state.writer = false;
        self.cvar.notify_all();
    }
}

#[derive(Debug, Default)]
struct PathLockState {
    readers: usize,
    writer: bool,
}

/// Owned read guard returned by [`FileLocker::lock_read`].
#[must_use]
#[derive(Debug)]
pub struct FileReadGuard {
    lock: Arc<PathLock>,
}

impl Drop for FileReadGuard {
    fn drop(&mut self) {
        self.lock.unlock_read();
    }
}

/// Owned write guard returned by [`FileLocker::lock_write`].
#[must_use]
#[derive(Debug)]
pub struct FileWriteGuard {
    lock: Arc<PathLock>,
}

impl Drop for FileWriteGuard {
    fn drop(&mut self) {
        self.lock.unlock_write();
    }
}

fn lock_unpoisoned<T>(mutex: &StdMutex<T>) -> StdMutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn wait_unpoisoned<'a, T>(cvar: &Condvar, guard: StdMutexGuard<'a, T>) -> StdMutexGuard<'a, T> {
    cvar.wait(guard).unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::FileLocker;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn read_locks_for_same_path_can_overlap() {
        let locker = Arc::new(FileLocker::new());
        let _read = locker.lock_read("world.vxr");
        let worker_locker = locker.clone();
        let (tx, rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            let _read = worker_locker.lock_read("world.vxr");
            tx.send(()).unwrap();
        });

        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn write_lock_waits_for_read_lock_on_same_path() {
        let locker = Arc::new(FileLocker::new());
        let read = locker.lock_read("world.vxr");
        let worker_locker = locker.clone();
        let (tx, rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            let _write = worker_locker.lock_write("world.vxr");
            tx.send(()).unwrap();
        });

        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(read);
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn writes_to_different_paths_do_not_block_each_other() {
        let locker = Arc::new(FileLocker::new());
        let _write = locker.lock_write("a.vxr");
        let worker_locker = locker.clone();
        let (tx, rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            let _write = worker_locker.lock_write("b.vxr");
            tx.send(()).unwrap();
        });

        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn repeated_locks_reuse_the_same_path_entry() {
        let locker = FileLocker::new();
        {
            let _a = locker.lock_read("world.vxr");
            let _b = locker.lock_read("world.vxr");
            assert_eq!(locker.tracked_path_count(), 1);
        }
        let _c = locker.lock_write("world.vxr");
        assert_eq!(locker.tracked_path_count(), 1);
    }
}
