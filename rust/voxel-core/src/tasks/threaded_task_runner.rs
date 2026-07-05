//! Minimal threaded task runner.
//!
//! Ported from `util/tasks/threaded_task_runner.{h,cpp}`. This implementation
//! keeps the core engine contract needed by stream tasks: priority picking,
//! serial-task gating, postponed requeueing, cooperative cancellation,
//! completed-task draining and idle waiting. Godot-specific debug/profiling
//! surfaces and hot resizing are intentionally deferred.

use super::{TaskPriority, TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

/// Generic thread pool for owned [`ThreadedTask`] objects.
pub struct ThreadedTaskRunner {
    shared: Arc<Shared>,
    handles: Vec<JoinHandle<()>>,
}

impl ThreadedTaskRunner {
    pub const MAX_THREADS: usize = 128;

    pub fn new(thread_count: usize) -> Self {
        let mut runner = Self {
            shared: Arc::new(Shared::default()),
            handles: Vec::new(),
        };
        runner.set_thread_count(thread_count);
        runner
    }

    pub fn set_thread_count(&mut self, count: usize) {
        if !self.handles.is_empty() {
            self.wait_for_all_tasks();
            self.stop_threads();
        }

        let count = count.min(Self::MAX_THREADS);
        {
            let mut state = self.shared.lock_state();
            state.stopping = false;
        }

        self.handles.reserve(count);
        for index in 0..count {
            let shared = self.shared.clone();
            self.handles
                .push(thread::spawn(move || worker_loop(shared, index as u8)));
        }
    }

    pub fn thread_count(&self) -> usize {
        self.handles.len()
    }

    pub fn enqueue(&self, task: Box<dyn ThreadedTask>, serial: bool) {
        let mut state = self.shared.lock_state();
        state.tasks.push(TaskItem {
            task,
            cached_priority: TaskPriority::min(),
            is_serial: serial,
        });
        self.shared.cvar.notify_all();
    }

    pub fn enqueue_many<I>(&self, tasks: I, serial: bool)
    where
        I: IntoIterator<Item = Box<dyn ThreadedTask>>,
    {
        let mut state = self.shared.lock_state();
        for task in tasks {
            state.tasks.push(TaskItem {
                task,
                cached_priority: TaskPriority::min(),
                is_serial: serial,
            });
        }
        self.shared.cvar.notify_all();
    }

    pub fn wait_for_all_tasks(&self) {
        let mut state = self.shared.lock_state();
        while state.has_pending_or_running_tasks() {
            state = self.shared.wait(state);
        }
    }

    pub fn drain_completed_tasks(&self) -> Vec<Box<dyn ThreadedTask>> {
        let mut state = self.shared.lock_state();
        std::mem::take(&mut state.completed_tasks)
    }

    /// Queued, postponed or running tasks. Completed-but-undrained tasks are
    /// not counted, matching the C++ debug remaining counter.
    pub fn remaining_task_count(&self) -> usize {
        let state = self.shared.lock_state();
        state.tasks.len() + state.spinning_tasks.len() + state.running_count
    }

    pub fn shutdown(&mut self) {
        if self.handles.is_empty() {
            return;
        }
        self.wait_for_all_tasks();
        self.stop_threads();
    }

    fn stop_threads(&mut self) {
        {
            let mut state = self.shared.lock_state();
            state.stopping = true;
            self.shared.cvar.notify_all();
        }

        for handle in self.handles.drain(..) {
            handle.join().expect("threaded task worker panicked");
        }
    }
}

impl Default for ThreadedTaskRunner {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Drop for ThreadedTaskRunner {
    fn drop(&mut self) {
        self.stop_threads();
    }
}

struct Shared {
    state: Mutex<RunnerState>,
    cvar: Condvar,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            state: Mutex::new(RunnerState::default()),
            cvar: Condvar::new(),
        }
    }
}

impl Shared {
    fn lock_state(&self) -> MutexGuard<'_, RunnerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, guard: MutexGuard<'a, RunnerState>) -> MutexGuard<'a, RunnerState> {
        self.cvar
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct RunnerState {
    tasks: Vec<TaskItem>,
    spinning_tasks: VecDeque<TaskItem>,
    completed_tasks: Vec<Box<dyn ThreadedTask>>,
    stopping: bool,
    running_count: usize,
    serial_running: bool,
    prefer_postponed_next: bool,
}

impl RunnerState {
    fn has_pending_or_running_tasks(&self) -> bool {
        !self.tasks.is_empty() || !self.spinning_tasks.is_empty() || self.running_count != 0
    }
}

struct TaskItem {
    task: Box<dyn ThreadedTask>,
    cached_priority: TaskPriority,
    is_serial: bool,
}

fn worker_loop(shared: Arc<Shared>, thread_index: u8) {
    loop {
        let item = {
            let mut state = shared.lock_state();
            loop {
                if state.stopping {
                    return;
                }

                if refresh_priorities_and_complete_cancelled(&mut state) {
                    shared.cvar.notify_all();
                }

                if let Some(item) = pick_next_task(&mut state) {
                    state.running_count += 1;
                    break item;
                }

                state = shared.wait(state);
            }
        };

        run_task_item(&shared, thread_index, item);
    }
}

fn refresh_priorities_and_complete_cancelled(state: &mut RunnerState) -> bool {
    let mut completed_cancelled = false;
    let mut i = 0;
    while i < state.tasks.len() {
        let item = &mut state.tasks[i];
        item.cached_priority = item.task.priority();
        if item.task.is_cancelled() {
            let item = state.tasks.swap_remove(i);
            state.completed_tasks.push(item.task);
            completed_cancelled = true;
            continue;
        }
        i += 1;
    }
    completed_cancelled
}

fn pick_next_task(state: &mut RunnerState) -> Option<TaskItem> {
    let prefer_postponed = state.prefer_postponed_next;
    let picked = if prefer_postponed {
        pick_postponed_task(state).or_else(|| pick_prioritized_task(state))
    } else {
        pick_prioritized_task(state).or_else(|| pick_postponed_task(state))
    };

    if let Some(item) = picked {
        state.prefer_postponed_next = !prefer_postponed;
        if item.is_serial {
            debug_assert!(!state.serial_running);
            state.serial_running = true;
        }
        return Some(item);
    }

    None
}

fn pick_postponed_task(state: &mut RunnerState) -> Option<TaskItem> {
    for i in 0..state.spinning_tasks.len() {
        if state.spinning_tasks[i].is_serial && state.serial_running {
            continue;
        }
        return state.spinning_tasks.remove(i);
    }
    None
}

fn pick_prioritized_task(state: &mut RunnerState) -> Option<TaskItem> {
    let mut best_index = None;
    let mut best_priority = TaskPriority::min();

    for (i, item) in state.tasks.iter().enumerate() {
        if item.is_serial && state.serial_running {
            continue;
        }
        if best_index.is_none() || item.cached_priority > best_priority {
            best_index = Some(i);
            best_priority = item.cached_priority;
        }
    }

    best_index.map(|index| state.tasks.swap_remove(index))
}

fn run_task_item(shared: &Shared, thread_index: u8, mut item: TaskItem) {
    let outcome = if item.task.is_cancelled() {
        TaskRunOutcome::Complete(item.task)
    } else {
        item.task
            .run(ThreadedTaskContext::new(thread_index, item.cached_priority))
    };

    let mut state = shared.lock_state();
    state.running_count -= 1;
    if item.is_serial {
        debug_assert!(state.serial_running);
        state.serial_running = false;
    }

    match outcome {
        TaskRunOutcome::Complete(task) => {
            state.completed_tasks.push(task);
        }
        TaskRunOutcome::Postponed(task) => {
            state.spinning_tasks.push_back(TaskItem {
                task,
                cached_priority: item.cached_priority,
                is_serial: item.is_serial,
            });
        }
        TaskRunOutcome::TakenOut => {}
    }

    shared.cvar.notify_all();
}

#[cfg(test)]
mod tests {
    use super::ThreadedTaskRunner;
    use crate::tasks::{TaskPriority, TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[derive(Default)]
    struct Counter {
        current: AtomicUsize,
        max: AtomicUsize,
        completed: AtomicUsize,
        applied: AtomicUsize,
    }

    impl Counter {
        fn enter(&self) {
            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            let mut previous = self.max.load(Ordering::SeqCst);
            while previous < current {
                match self.max.compare_exchange_weak(
                    previous,
                    current,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(next) => previous = next,
                }
            }
        }

        fn leave(&self) {
            self.current.fetch_sub(1, Ordering::SeqCst);
            self.completed.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct CountingTask {
        counter: Arc<Counter>,
        sleep: Duration,
        completed: bool,
    }

    impl CountingTask {
        fn new(counter: Arc<Counter>, sleep: Duration) -> Self {
            Self {
                counter,
                sleep,
                completed: false,
            }
        }
    }

    impl ThreadedTask for CountingTask {
        fn run(mut self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            self.counter.enter();
            thread::sleep(self.sleep);
            self.counter.leave();
            self.completed = true;
            TaskRunOutcome::Complete(self)
        }

        fn apply_result(self: Box<Self>) {
            assert!(self.completed);
            self.counter.applied.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct PriorityTask {
        priority: TaskPriority,
        id: usize,
        order: Arc<Mutex<Vec<usize>>>,
    }

    impl ThreadedTask for PriorityTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            self.order.lock().unwrap().push(self.id);
            TaskRunOutcome::Complete(self)
        }

        fn priority(&mut self) -> TaskPriority {
            self.priority
        }
    }

    struct CancelledTask {
        ran: Arc<AtomicBool>,
        applied: Arc<AtomicBool>,
    }

    impl ThreadedTask for CancelledTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            self.ran.store(true, Ordering::SeqCst);
            TaskRunOutcome::Complete(self)
        }

        fn apply_result(self: Box<Self>) {
            self.applied.store(true, Ordering::SeqCst);
        }

        fn is_cancelled(&mut self) -> bool {
            true
        }
    }

    struct PostponedTask {
        attempts: Arc<AtomicUsize>,
    }

    impl ThreadedTask for PostponedTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                TaskRunOutcome::Postponed(self)
            } else {
                TaskRunOutcome::Complete(self)
            }
        }
    }

    struct TakenOutTask;

    impl ThreadedTask for TakenOutTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            TaskRunOutcome::TakenOut
        }
    }

    fn apply_all(tasks: Vec<Box<dyn ThreadedTask>>) {
        for task in tasks {
            task.apply_result();
        }
    }

    #[test]
    fn parallel_tasks_run_and_completed_tasks_are_drained_explicitly() {
        let runner = ThreadedTaskRunner::new(4);
        let counter = Arc::new(Counter::default());

        for _ in 0..8 {
            runner.enqueue(
                Box::new(CountingTask::new(
                    counter.clone(),
                    Duration::from_millis(10),
                )),
                false,
            );
        }

        runner.wait_for_all_tasks();
        assert_eq!(counter.completed.load(Ordering::SeqCst), 8);
        assert!(counter.max.load(Ordering::SeqCst) <= 4);
        assert_eq!(counter.applied.load(Ordering::SeqCst), 0);

        apply_all(runner.drain_completed_tasks());
        assert_eq!(counter.applied.load(Ordering::SeqCst), 8);
        assert!(runner.drain_completed_tasks().is_empty());
    }

    #[test]
    fn serial_tasks_do_not_overlap_even_with_multiple_threads() {
        let runner = ThreadedTaskRunner::new(4);
        let counter = Arc::new(Counter::default());

        for _ in 0..8 {
            runner.enqueue(
                Box::new(CountingTask::new(counter.clone(), Duration::from_millis(5))),
                true,
            );
        }

        runner.wait_for_all_tasks();
        apply_all(runner.drain_completed_tasks());

        assert_eq!(counter.completed.load(Ordering::SeqCst), 8);
        assert_eq!(counter.max.load(Ordering::SeqCst), 1);
        assert_eq!(counter.current.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn highest_priority_task_runs_first_when_worker_starts_after_enqueue() {
        let mut runner = ThreadedTaskRunner::new(0);
        let order = Arc::new(Mutex::new(Vec::new()));

        runner.enqueue(
            Box::new(PriorityTask {
                priority: TaskPriority::new(1, 0, 0, 0),
                id: 1,
                order: order.clone(),
            }),
            false,
        );
        runner.enqueue(
            Box::new(PriorityTask {
                priority: TaskPriority::new(0, 0, 1, 0),
                id: 2,
                order: order.clone(),
            }),
            false,
        );

        runner.set_thread_count(1);
        runner.wait_for_all_tasks();

        assert_eq!(*order.lock().unwrap(), vec![2, 1]);
    }

    #[test]
    fn cancelled_tasks_are_completed_without_running() {
        let runner = ThreadedTaskRunner::new(1);
        let ran = Arc::new(AtomicBool::new(false));
        let applied = Arc::new(AtomicBool::new(false));

        runner.enqueue(
            Box::new(CancelledTask {
                ran: ran.clone(),
                applied: applied.clone(),
            }),
            false,
        );

        runner.wait_for_all_tasks();
        apply_all(runner.drain_completed_tasks());

        assert!(!ran.load(Ordering::SeqCst));
        assert!(applied.load(Ordering::SeqCst));
    }

    #[test]
    fn postponed_tasks_are_requeued_until_complete() {
        let runner = ThreadedTaskRunner::new(1);
        let attempts = Arc::new(AtomicUsize::new(0));

        runner.enqueue(
            Box::new(PostponedTask {
                attempts: attempts.clone(),
            }),
            false,
        );

        runner.wait_for_all_tasks();
        apply_all(runner.drain_completed_tasks());

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn taken_out_tasks_are_not_completed() {
        let runner = ThreadedTaskRunner::new(1);

        runner.enqueue(Box::new(TakenOutTask), false);
        runner.wait_for_all_tasks();

        assert!(runner.drain_completed_tasks().is_empty());
        assert_eq!(runner.remaining_task_count(), 0);
    }

    #[test]
    fn shutdown_waits_for_running_tasks_and_joins_workers() {
        let mut runner = ThreadedTaskRunner::new(1);
        let counter = Arc::new(Counter::default());

        runner.enqueue(
            Box::new(CountingTask::new(
                counter.clone(),
                Duration::from_millis(10),
            )),
            false,
        );

        runner.shutdown();
        assert_eq!(runner.thread_count(), 0);
        assert_eq!(counter.completed.load(Ordering::SeqCst), 1);
        apply_all(runner.drain_completed_tasks());
        assert_eq!(counter.applied.load(Ordering::SeqCst), 1);
    }
}
