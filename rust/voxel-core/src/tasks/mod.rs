//! Task utilities ported from `util/tasks/`.
//!
//! This module intentionally stops at engine-agnostic value types. Task
//! runners, queues and Godot-facing scheduling come later with the streaming
//! layer.

pub mod cancellation_token;
pub mod task_priority;

pub use cancellation_token::TaskCancellationToken;
pub use task_priority::TaskPriority;
