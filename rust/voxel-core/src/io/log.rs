//! Logging primitives.
//!
//! Ported from `util/io/log.{h,cpp}`. The C++ version delegates to Godot's
//! `OS`/`UtilityFunctions` print routines; `voxel-core` is engine-agnostic, so
//! here we route to `eprintln!` (errors/warnings) and `println!` (lines), gated
//! by a global verbose flag. The `voxel-gdext` crate can override these to call
//! into Godot's logger when it links the engine.
//!
//! Not ported: `ZN_DO_ONCE` (use `std::sync::Once` directly), and the
//! `ZN_DEBUG_LOG_FILE_ENABLED` file-redirect (debug-only, rarely used).

use core::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Enable/disable verbose logging globally. Matches enabling Godot's
/// `--verbose` flag.
#[inline]
pub fn set_verbose_enabled(enabled: bool) {
    VERBOSE.store(enabled, Ordering::Relaxed);
}

/// Whether verbose output is on. Matches `is_verbose_output_enabled`.
#[inline]
pub fn is_verbose_output_enabled() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Print a line to stdout. Matches `print_line`.
#[inline]
pub fn print_line(msg: &str) {
    println!("{msg}");
}

/// Print a warning to stderr with location info. Matches `ZN_PRINT_WARNING`.
#[inline]
pub fn print_warning(msg: &str, func: &str, file: &str, line: u32) {
    eprintln!("WARNING: {msg}\n   at: {func} ({file}:{line})");
}

/// Print an error to stderr with location info. Matches `ZN_PRINT_ERROR`.
#[inline]
pub fn print_error(msg: &str, func: &str, file: &str, line: u32) {
    eprintln!("ERROR: {msg}\n   at: {func} ({file}:{line})");
}

/// Flush stdout. Matches `flush_stdout`.
#[inline]
pub fn flush_stdout() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Print only when verbose mode is on. Replaces the `ZN_PRINT_VERBOSE` macro.
/// Takes a closure so the message is never formatted when verbose is off.
#[inline]
pub fn print_verbose<F: FnOnce() -> String>(msg: F) {
    if is_verbose_output_enabled() {
        print_line(&msg());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbose_flag_round_trip() {
        let prev = is_verbose_output_enabled();
        set_verbose_enabled(true);
        assert!(is_verbose_output_enabled());
        set_verbose_enabled(false);
        assert!(!is_verbose_output_enabled());
        set_verbose_enabled(prev);
    }

    // We don't assert on stdout/stderr contents (they're captured by the test
    // harness inconsistently); the functions are exercised for panic-freedom.
    #[test]
    fn print_helpers_dont_panic() {
        print_line("test line");
        print_warning("test warning", "func", "file.rs", 10);
        print_error("test error", "func", "file.rs", 11);
        flush_stdout();
    }

    #[test]
    fn print_verbose_respects_flag() {
        set_verbose_enabled(false);
        // Closure would only run if verbose; with it off, msg is never built.
        print_verbose(|| panic!("should not be called when verbose is off"));
        set_verbose_enabled(true);
        print_verbose(|| "called when verbose".to_string());
    }
}
