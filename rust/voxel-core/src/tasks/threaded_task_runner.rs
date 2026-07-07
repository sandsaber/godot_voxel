//! Minimal threaded task runner.
//!
//! Ported from `util/tasks/threaded_task_runner.{h,cpp}`. This implementation
//! keeps the core engine contract needed by stream tasks: priority picking,
//! serial-task gating, postponed requeueing, cooperative cancellation,
//! completed-task draining and idle waiting. Godot-specific debug/profiling
//! surfaces and hot resizing are intentionally deferred.

use super::{TaskPriority, TaskRunOutcome, ThreadedTask, ThreadedTaskContext};
use crate::thread::Semaphore;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Default priority-recompute period. Matches the C++ runner
/// (`_priority_update_period_ms = 32`): cached priorities and cancellation
/// drains run at most every 32 ms, so a worker wake does no per-task
/// `priority()`/`is_cancelled()` virtual dispatches in the common case.
const DEFAULT_PRIORITY_UPDATE_PERIOD: Duration = Duration::from_millis(32);

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
        // Default throttle matches C++ (`_priority_update_period_ms = 32`).
        runner.shared.lock_state().priority_update_period = DEFAULT_PRIORITY_UPDATE_PERIOD;
        runner.set_thread_count(thread_count);
        runner
    }

    /// Sets how often cached priorities are recomputed and cancelled tasks
    /// drained, mirroring `ThreadedTaskRunner::set_priority_update_period`.
    /// Smaller periods make the runner more responsive to priority changes
    /// (e.g. a moving viewer); larger periods reduce per-task virtual
    /// dispatches under heavy queue pressure. Setting to `Duration::ZERO`
    /// disables throttling and recomputes on every worker wake.
    pub fn set_priority_update_period(&self, period: Duration) {
        self.shared.lock_state().priority_update_period = period;
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
        let mut staged_tasks = self.shared.lock_staged_tasks();
        staged_tasks.push(TaskItem {
            task,
            cached_priority: TaskPriority::min(),
            is_serial: serial,
        });
        drop(staged_tasks);
        self.shared.work_semaphore.post();
    }

    pub fn enqueue_many<I>(&self, tasks: I, serial: bool)
    where
        I: IntoIterator<Item = Box<dyn ThreadedTask>>,
    {
        let mut staged_tasks = self.shared.lock_staged_tasks();
        let mut count = 0;
        for task in tasks {
            staged_tasks.push(TaskItem {
                task,
                cached_priority: TaskPriority::min(),
                is_serial: serial,
            });
            count += 1;
        }
        drop(staged_tasks);

        for _ in 0..count {
            self.shared.work_semaphore.post();
        }
    }

    pub fn wait_for_all_tasks(&self) {
        let mut state = self.shared.lock_state();
        while state.has_pending_or_running_tasks() || self.shared.has_staged_tasks() {
            state = self.shared.wait(state);
        }
    }

    pub fn drain_completed_tasks(&self) -> Vec<Box<dyn ThreadedTask>> {
        let mut state = self.shared.lock_state();
        std::mem::take(&mut state.completed_tasks)
    }

    pub fn drain_completed_tasks_and_enqueue_followups(
        &self,
        followups_are_serial: bool,
    ) -> Vec<Box<dyn ThreadedTask>> {
        let mut completed_tasks = self.drain_completed_tasks();
        let mut followup_tasks = Vec::new();
        for task in &mut completed_tasks {
            followup_tasks.extend(task.take_follow_up_tasks());
        }
        if !followup_tasks.is_empty() {
            self.enqueue_many(followup_tasks, followups_are_serial);
        }
        completed_tasks
    }

    /// Queued, postponed or running tasks. Completed-but-undrained tasks are
    /// not counted, matching the C++ debug remaining counter.
    pub fn remaining_task_count(&self) -> usize {
        let state = self.shared.lock_state();
        let staged_tasks = self.shared.lock_staged_tasks();
        staged_tasks.len() + state.tasks.len() + state.spinning_tasks.len() + state.running_count
    }

    pub fn shutdown(&mut self) {
        if self.handles.is_empty() {
            return;
        }
        self.wait_for_all_tasks();
        self.stop_threads();
    }

    fn stop_threads(&mut self) {
        let thread_count = self.handles.len();
        {
            let mut state = self.shared.lock_state();
            state.stopping = true;
            self.shared.cvar.notify_all();
        }

        for _ in 0..thread_count {
            self.shared.work_semaphore.post();
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
    staged_tasks: Mutex<Vec<TaskItem>>,
    work_semaphore: Semaphore,
    cvar: Condvar,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            state: Mutex::new(RunnerState::default()),
            staged_tasks: Mutex::new(Vec::new()),
            work_semaphore: Semaphore::new(),
            cvar: Condvar::new(),
        }
    }
}

impl Shared {
    fn lock_state(&self) -> MutexGuard<'_, RunnerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_staged_tasks(&self) -> MutexGuard<'_, Vec<TaskItem>> {
        self.staged_tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn has_staged_tasks(&self) -> bool {
        !self.lock_staged_tasks().is_empty()
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
    tasks_sorted: bool,
    spinning_tasks: VecDeque<TaskItem>,
    completed_tasks: Vec<Box<dyn ThreadedTask>>,
    stopping: bool,
    running_count: usize,
    serial_running: bool,
    prefer_postponed_next: bool,
    /// Last time priorities were recomputed and cancelled tasks were drained.
    /// Mirrors `_last_priority_update_time_ms` in the C++ runner.
    last_priority_update: Option<Instant>,
    /// Period of the priority/cancellation refresh. Defaults to 32 ms (matching
    /// C++); exposed via [`ThreadedTaskRunner::set_priority_update_period`].
    priority_update_period: Duration,
}

impl RunnerState {
    fn has_pending_or_running_tasks(&self) -> bool {
        !self.tasks.is_empty() || !self.spinning_tasks.is_empty() || self.running_count != 0
    }

    fn has_queued_tasks(&self) -> bool {
        !self.tasks.is_empty() || !self.spinning_tasks.is_empty()
    }

    /// Returns true when the throttle window has elapsed since the last
    /// priority refresh. Always true on the first refresh so initial picks
    /// don't run with stale `TaskPriority::min()` cache values.
    fn priority_refresh_due(&self, now: Instant) -> bool {
        match self.last_priority_update {
            None => true,
            Some(last) => now.duration_since(last) >= self.priority_update_period,
        }
    }
}

struct TaskItem {
    task: Box<dyn ThreadedTask>,
    cached_priority: TaskPriority,
    is_serial: bool,
}

fn worker_loop(shared: Arc<Shared>, thread_index: u8) {
    loop {
        shared.work_semaphore.wait();

        let item = {
            let mut state = shared.lock_state();
            if state.stopping {
                return;
            }

            if drain_staged_tasks(&shared, &mut state) {
                state.tasks_sorted = false;
            }

            let now = Instant::now();
            if !state.tasks_sorted || state.priority_refresh_due(now) {
                if refresh_priorities_and_complete_cancelled(&mut state) {
                    shared.cvar.notify_all();
                }
                state.last_priority_update = Some(now);
            }

            let Some(item) = pick_next_task(&mut state) else {
                continue;
            };

            state.running_count += 1;
            item
        };

        run_task_item(&shared, thread_index, item);
    }
}

fn drain_staged_tasks(shared: &Shared, state: &mut RunnerState) -> bool {
    let mut staged_tasks = shared.lock_staged_tasks();
    if staged_tasks.is_empty() {
        return false;
    }

    state.tasks.extend(staged_tasks.drain(..));
    true
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
    state
        .tasks
        .sort_unstable_by_key(|item| item.cached_priority);
    state.tasks_sorted = true;
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
    if !state.serial_running {
        return state.tasks.pop();
    }

    state
        .tasks
        .iter()
        .rposition(|item| !item.is_serial)
        .map(|index| state.tasks.remove(index))
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

    let mut should_post_work = false;
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
            should_post_work = true;
        }
        TaskRunOutcome::TakenOut => {}
    }

    should_post_work |= item.is_serial && state.has_queued_tasks();
    shared.cvar.notify_all();
    drop(state);

    if should_post_work {
        shared.work_semaphore.post();
    }
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

    struct FollowUpParentTask {
        order: Arc<Mutex<Vec<&'static str>>>,
        follow_up_tasks: Vec<Box<dyn ThreadedTask>>,
    }

    impl ThreadedTask for FollowUpParentTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            self.order.lock().unwrap().push("parent");
            TaskRunOutcome::Complete(self)
        }

        fn take_follow_up_tasks(&mut self) -> Vec<Box<dyn ThreadedTask>> {
            std::mem::take(&mut self.follow_up_tasks)
        }
    }

    struct OrderedTask {
        name: &'static str,
        order: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ThreadedTask for OrderedTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            self.order.lock().unwrap().push(self.name);
            TaskRunOutcome::Complete(self)
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
    fn enqueue_does_not_block_on_worker_queue_lock() {
        let runner = ThreadedTaskRunner::new(0);
        let state_guard = runner.shared.lock_state();
        let enqueued = Arc::new(AtomicBool::new(false));

        thread::scope(|scope| {
            let thread_enqueued = enqueued.clone();
            let runner_ref = &runner;
            scope.spawn(move || {
                runner_ref.enqueue(Box::new(TakenOutTask), false);
                thread_enqueued.store(true, Ordering::SeqCst);
            });

            thread::sleep(Duration::from_millis(50));
            let completed_while_state_was_locked = enqueued.load(Ordering::SeqCst);
            drop(state_guard);

            assert!(
                completed_while_state_was_locked,
                "enqueue should use a staging queue instead of blocking on the worker queue lock"
            );
        });
    }

    #[test]
    fn follow_up_tasks_are_enqueued_when_completed_tasks_are_drained() {
        let runner = ThreadedTaskRunner::new(1);
        let order = Arc::new(Mutex::new(Vec::new()));

        runner.enqueue(
            Box::new(FollowUpParentTask {
                order: order.clone(),
                follow_up_tasks: vec![Box::new(OrderedTask {
                    name: "child",
                    order: order.clone(),
                })],
            }),
            false,
        );
        runner.wait_for_all_tasks();

        apply_all(runner.drain_completed_tasks_and_enqueue_followups(false));
        runner.wait_for_all_tasks();
        apply_all(runner.drain_completed_tasks_and_enqueue_followups(false));

        assert_eq!(*order.lock().unwrap(), vec!["parent", "child"]);
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

    #[test]
    fn priority_recompute_is_throttle_within_the_update_period_single_worker() {
        // With a long priority-update period, the runner must NOT call
        // `priority()` on every worker wake: cached priorities are reused
        // across wakes until the period elapses. This mirrors the C++
        // `_priority_update_period_ms` throttle.
        //
        // Uses a single worker so the count is deterministic: the worker
        // wakes, runs the initial refresh (one priority() per task), picks a
        // task, runs it, returns to the loop. Without throttling it would
        // re-run the refresh on every wake (16+ priority() calls across
        // iterations); with throttling only the initial 16 happen.
        struct PriorityCountTask {
            priority_calls: Arc<AtomicUsize>,
        }
        impl ThreadedTask for PriorityCountTask {
            fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
                TaskRunOutcome::Complete(self)
            }
            fn priority(&mut self) -> TaskPriority {
                self.priority_calls.fetch_add(1, Ordering::SeqCst);
                TaskPriority::max()
            }
        }

        let runner = ThreadedTaskRunner::new(1);
        runner.set_priority_update_period(Duration::from_secs(60));

        let priority_calls = Arc::new(AtomicUsize::new(0));
        for _ in 0..16 {
            runner.enqueue(
                Box::new(PriorityCountTask {
                    priority_calls: priority_calls.clone(),
                }),
                false,
            );
        }

        runner.wait_for_all_tasks();
        apply_all(runner.drain_completed_tasks());

        // Single worker + 60 s window ⇒ exactly one priority() per task.
        assert_eq!(
            priority_calls.load(Ordering::SeqCst),
            16,
            "throttled runner should call priority() exactly once per task"
        );
    }
}
