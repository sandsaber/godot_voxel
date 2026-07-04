//! Shared cancellation token ported from `util/tasks/cancellation_token.h`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared task cancellation flag.
///
/// A default token is invalid, matching the C++ null shared pointer state.
/// Invalid tokens are treated as not cancelled; `cancel()` on an invalid token
/// is a no-op so this Result-free helper stays ergonomic in tests and task
/// plumbing.
#[derive(Debug, Clone, Default)]
pub struct TaskCancellationToken {
    cancelled: Option<Arc<AtomicBool>>,
}

impl TaskCancellationToken {
    pub fn create() -> Self {
        Self {
            cancelled: Some(Arc::new(AtomicBool::new(false))),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.cancelled.is_some()
    }

    pub fn cancel(&self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.store(true, Ordering::SeqCst);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
            .as_ref()
            .is_some_and(|cancelled| cancelled.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::TaskCancellationToken;

    #[test]
    fn default_token_is_invalid_and_not_cancelled() {
        let token = TaskCancellationToken::default();
        assert!(!token.is_valid());
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn created_token_starts_active() {
        let token = TaskCancellationToken::create();
        assert!(token.is_valid());
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancellation_is_shared_between_clones() {
        let token = TaskCancellationToken::create();
        let clone = token.clone();

        clone.cancel();

        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
    }
}
