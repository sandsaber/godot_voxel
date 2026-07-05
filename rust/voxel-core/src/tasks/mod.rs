//! Task utilities ported from `util/tasks/`.
//!
//! Phase 4 includes the engine-agnostic task runner used by streaming. Godot
//! scheduler bindings, profiling/debug UI and terrain-specific task types live
//! in later layers.

pub mod async_dependency_tracker;
pub mod cancellation_token;
pub mod task_priority;
pub mod threaded_task;
pub mod threaded_task_runner;

pub use async_dependency_tracker::{
    AsyncDependencyCompletion, AsyncDependencyError, AsyncDependencyTracker,
};
pub use cancellation_token::TaskCancellationToken;
pub use task_priority::TaskPriority;
pub use threaded_task::{TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
pub use threaded_task_runner::ThreadedTaskRunner;
