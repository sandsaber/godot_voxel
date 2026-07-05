//! Owned threaded task contract.
//!
//! Ported from `util/tasks/threaded_task.h`. Rust tasks are owned by the
//! runner as `Box<dyn ThreadedTask>` and `run` consumes/returns that ownership,
//! which lets `TakenOut` model the C++ "runner drops the pointer because
//! another system re-scheduled it" status without unsafe raw pointers.

use super::TaskPriority;

/// Context passed to a task while it runs on a worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadedTaskContext {
    pub thread_index: u8,
    pub task_priority: TaskPriority,
}

impl ThreadedTaskContext {
    pub const fn new(thread_index: u8, task_priority: TaskPriority) -> Self {
        Self {
            thread_index,
            task_priority,
        }
    }
}

/// Outcome returned by [`ThreadedTask::run`].
pub enum TaskRunOutcome {
    /// The task finished and should be exposed through the completed queue.
    Complete(Box<dyn ThreadedTask>),
    /// The task should be run again later.
    Postponed(Box<dyn ThreadedTask>),
    /// The task transferred ownership elsewhere and must not be completed.
    TakenOut,
}

/// Task runnable by [`super::ThreadedTaskRunner`].
pub trait ThreadedTask: Send + 'static {
    /// Runs on a worker thread. Implementations must return `self` in
    /// [`TaskRunOutcome::Complete`] or [`TaskRunOutcome::Postponed`] unless
    /// ownership was intentionally transferred elsewhere.
    fn run(self: Box<Self>, ctx: ThreadedTaskContext) -> TaskRunOutcome;

    /// Runs on the caller/main side after draining completed tasks.
    fn apply_result(self: Box<Self>) {}

    /// Follow-up tasks produced by this task while running.
    ///
    /// Callers should drain this before [`ThreadedTask::apply_result`] if they
    /// need to enqueue dependent work.
    fn take_follow_up_tasks(&mut self) -> Vec<Box<dyn ThreadedTask>> {
        Vec::new()
    }

    /// Dynamic priority. Higher packed values run first.
    fn priority(&mut self) -> TaskPriority {
        TaskPriority::max()
    }

    /// Cooperative cancellation. Cancelled tasks skip `run` and still enter
    /// the completed queue so callers can apply/drop them deterministically.
    fn is_cancelled(&mut self) -> bool {
        false
    }

    /// Static debug name for diagnostics.
    fn debug_name(&self) -> &'static str {
        "<unnamed>"
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
    use crate::tasks::TaskPriority;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct CompleteTask {
        ran: Arc<AtomicBool>,
    }

    impl ThreadedTask for CompleteTask {
        fn run(self: Box<Self>, ctx: ThreadedTaskContext) -> TaskRunOutcome {
            assert_eq!(ctx.thread_index, 7);
            assert_eq!(ctx.task_priority, TaskPriority::new(1, 2, 3, 4));
            self.ran.store(true, Ordering::SeqCst);
            TaskRunOutcome::Complete(self)
        }
    }

    #[test]
    fn task_context_carries_thread_index_and_priority() {
        let ran = Arc::new(AtomicBool::new(false));
        let task = Box::new(CompleteTask { ran: ran.clone() });
        let outcome = task.run(ThreadedTaskContext::new(7, TaskPriority::new(1, 2, 3, 4)));

        assert!(matches!(outcome, TaskRunOutcome::Complete(_)));
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn default_task_metadata_matches_cpp_contract() {
        let mut task: Box<dyn ThreadedTask> = Box::new(CompleteTask {
            ran: Arc::new(AtomicBool::new(false)),
        });

        assert_eq!(task.priority(), TaskPriority::max());
        assert!(!task.is_cancelled());
        assert_eq!(task.debug_name(), "<unnamed>");
    }
}
