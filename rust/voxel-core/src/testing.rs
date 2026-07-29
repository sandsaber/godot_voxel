//! Testing helpers.
//!
//! Ported from `util/testing/`:
//! - [`TestDirectory`] — RAII temp directory (from `test_directory.h`).
//! - [`TestOptions`] — include/exclude test-name filters (from `test_options.h`).
//!
//! ## `test_macros.h` → native Rust
//!
//! The C++ macros `ZN_TEST_ASSERT` / `ZN_TEST_ASSERT_V` / `ZN_TEST_ASSERT_MSG`
//! print an error and trap. Rust's `assert!` / `assert_eq!` / `panic!` cover this
//! exactly (and print location automatically), so no port is needed — just use
//! the native macros.
//!
//! | C++ | Rust |
//! |-----|------|
//! | `ZN_TEST_ASSERT(cond)` | `assert!(cond)` |
//! | `ZN_TEST_ASSERT_MSG(cond, msg)` | `assert!(cond, "{}", msg)` |
//! | `ZN_TEST_ASSERT_V(cond, val)` | `assert!(cond)` (return value not needed — Rust unwinds) |

use std::path::{Path, PathBuf};

/// A temporary directory that is created on construction and removed (recursively)
/// on drop. Matches `zylann::testing::TestDirectory`.
///
/// The directory lives under the OS temp dir with a unique name. Use
/// [`path`](Self::path) to get its location for fixtures.
pub struct TestDirectory {
    path: PathBuf,
    valid: bool,
}

impl TestDirectory {
    /// Create a fresh, empty temp directory.
    pub fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(unique_dir_name());
        std::fs::create_dir(&path)?;
        Ok(Self { path, valid: true })
    }

    /// Whether the directory was successfully created.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Path to the temp directory. Matches `get_path`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consume without removing the directory on drop (e.g. to inspect fixtures
    /// after a test). Matches leaving `_valid = false` in C++ before destruction.
    pub fn leak(mut self) -> PathBuf {
        self.valid = false;
        self.path.clone()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.valid {
            // Best-effort removal; ignore errors (the temp dir is, by definition,
            // disposable). The C++ destructor also swallows removal failures.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

impl Default for TestDirectory {
    fn default() -> Self {
        Self::new().expect("failed to create TestDirectory")
    }
}

fn unique_dir_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("voxel_core_test_{pid}_{n}")
}

/// Include/exclude test-name filters. Matches `zylann::testing::TestOptions`.
///
/// A test runs iff its name is in `includes` (if non-empty) AND not in
/// `excludes`. Empty `includes` means "all allowed unless excluded".
pub struct TestOptions {
    includes: Vec<String>,
    excludes: Vec<String>,
}

impl TestOptions {
    /// No filters — everything runs.
    pub fn new() -> Self {
        Self {
            includes: Vec::new(),
            excludes: Vec::new(),
        }
    }

    /// Build from explicit include/exclude lists. Matches the C++ ctor that
    /// parses these out of a Godot `Dictionary` (the dictionary parsing itself
    /// belongs to the `voxel-gdext` layer, not engine-agnostic `voxel-core`).
    pub fn from_lists(includes: Vec<String>, excludes: Vec<String>) -> Self {
        Self { includes, excludes }
    }

    /// True if `test_name` is permitted by the filters. Matches `can_run`.
    pub fn can_run(&self, test_name: &str) -> bool {
        if !self.includes.is_empty() && !self.includes.iter().any(|n| n == test_name) {
            return false;
        }
        if self.excludes.iter().any(|n| n == test_name) {
            return false;
        }
        true
    }

    /// Like [`can_run`](Self::can_run) but also prints a skip message when the
    /// test is filtered out. Matches `can_run_print`.
    pub fn can_run_print(&self, test_name: &str) -> bool {
        let ok = self.can_run(test_name);
        if !ok {
            println!("Skipped test: {test_name}");
        }
        ok
    }
}

impl Default for TestOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_directory_create_and_remove() {
        let dir = TestDirectory::new().expect("create");
        assert!(dir.is_valid());
        assert!(dir.path().exists());
        assert!(dir.path().is_dir());
        let p = dir.path().to_path_buf();
        drop(dir);
        assert!(!p.exists(), "temp dir should be removed on drop");
    }

    #[test]
    fn test_directory_leak_keeps_dir() {
        let dir = TestDirectory::new().expect("create");
        let p = dir.leak();
        assert!(p.exists(), "leaked dir must survive drop");
        // Clean up manually so the test is tidy.
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn test_directory_can_hold_files() {
        let dir = TestDirectory::new().expect("create");
        let f = dir.path().join("fixture.txt");
        std::fs::write(&f, b"hi").expect("write");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hi");
        // drop removes it recursively.
    }

    #[test]
    fn options_default_allows_all() {
        let opts = TestOptions::new();
        assert!(opts.can_run("anything"));
        assert!(opts.can_run("foo"));
    }

    #[test]
    fn options_includes_act_as_allowlist() {
        let opts = TestOptions::from_lists(vec!["a".into(), "b".into()], vec![]);
        assert!(opts.can_run("a"));
        assert!(opts.can_run("b"));
        assert!(!opts.can_run("c"));
    }

    #[test]
    fn options_excludes_block() {
        let opts = TestOptions::from_lists(vec![], vec!["bad".into()]);
        assert!(!opts.can_run("bad"));
        assert!(opts.can_run("good"));
    }

    #[test]
    fn options_include_plus_exclude_combine() {
        // Included but also excluded -> excluded wins.
        let opts = TestOptions::from_lists(vec!["a".into()], vec!["a".into()]);
        assert!(!opts.can_run("a"));
    }
}
