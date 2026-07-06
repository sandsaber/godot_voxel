//! Threaded stream save task ported from `streams/save_block_data_task.*`.

use crate::constants::voxel_constants::{TASK_PRIORITY_BAND3_DEFAULT, TASK_PRIORITY_SAVE_BAND2};
use crate::engine::StreamingDependency;
use crate::math::Vector3i;
use crate::storage::VoxelBuffer;
use crate::streams::{BlockDataOutput, VoxelSaveQuery, VoxelStreamError};
use crate::tasks::{
    AsyncDependencyError, AsyncDependencyTracker, TaskPriority, TaskRunOutcome, ThreadedTask,
    ThreadedTaskContext,
};
use std::sync::Arc;

pub struct SaveBlockDataTask {
    position_in_blocks: Vector3i,
    lod_index: u8,
    voxels: Option<VoxelBuffer>,
    stream_dependency: Arc<StreamingDependency>,
    tracker: Option<Arc<AsyncDependencyTracker>>,
    flush_on_last_tracked_task: bool,
    has_run: bool,
    had_voxels: bool,
    stream_error: Option<VoxelStreamError>,
    tracker_error: Option<AsyncDependencyError>,
    follow_up_tasks: Vec<Box<dyn ThreadedTask>>,
    output: Option<BlockDataOutput>,
}

impl SaveBlockDataTask {
    pub fn new_voxels(
        position_in_blocks: Vector3i,
        lod_index: u8,
        voxels: Option<VoxelBuffer>,
        stream_dependency: Arc<StreamingDependency>,
        tracker: Option<Arc<AsyncDependencyTracker>>,
        flush_on_last_tracked_task: bool,
    ) -> Self {
        let had_voxels = voxels.is_some();
        Self {
            position_in_blocks,
            lod_index,
            voxels,
            stream_dependency,
            tracker,
            flush_on_last_tracked_task,
            has_run: false,
            had_voxels,
            stream_error: None,
            tracker_error: None,
            follow_up_tasks: Vec::new(),
            output: None,
        }
    }

    pub const fn position_in_blocks(&self) -> Vector3i {
        self.position_in_blocks
    }

    pub const fn lod_index(&self) -> u8 {
        self.lod_index
    }

    pub const fn has_run(&self) -> bool {
        self.has_run
    }

    pub const fn had_voxels(&self) -> bool {
        self.had_voxels
    }

    pub fn stream_error(&self) -> Option<&VoxelStreamError> {
        self.stream_error.as_ref()
    }

    pub const fn tracker_error(&self) -> Option<AsyncDependencyError> {
        self.tracker_error
    }

    pub fn take_output(&mut self) -> Option<BlockDataOutput> {
        self.output.take()
    }

    fn run_save(&mut self) {
        self.output = None;
        let Some(voxels) = self.voxels.take() else {
            // Mirror the C++ apply_result contract: an aborted save (no voxels
            // to write) still emits a `Saved` output with `dropped = true` so
            // the caller knows the block was not persisted. `_has_run` stays
            // false in C++; here we surface that via the dropped flag.
            if let Some(tracker) = &self.tracker {
                tracker.abort();
            }
            self.output = Some(BlockDataOutput::saved_dropped(
                self.position_in_blocks,
                self.lod_index,
                self.had_voxels,
            ));
            return;
        };

        let stream = self.stream_dependency.stream();
        if let Err(error) = stream.save_voxel_block(VoxelSaveQuery::new(
            &voxels,
            self.position_in_blocks,
            self.lod_index,
        )) {
            self.stream_error = Some(error);
        }

        if let Some(tracker) = &self.tracker {
            match tracker.post_complete() {
                Ok(completion) => {
                    if self.flush_on_last_tracked_task && completion.was_last {
                        if let Err(error) = stream.flush() {
                            self.stream_error = Some(error);
                        }
                    }
                    self.follow_up_tasks = completion.next_tasks;
                }
                Err(error) => {
                    self.tracker_error = Some(error);
                }
            }
        }

        self.has_run = true;
        self.output = Some(if self.stream_error.is_some() {
            BlockDataOutput::saved_dropped(self.position_in_blocks, self.lod_index, self.had_voxels)
        } else {
            BlockDataOutput::saved(self.position_in_blocks, self.lod_index, self.had_voxels)
        });
    }
}

impl ThreadedTask for SaveBlockDataTask {
    fn run(mut self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
        self.run_save();
        TaskRunOutcome::Complete(self)
    }

    fn take_follow_up_tasks(&mut self) -> Vec<Box<dyn ThreadedTask>> {
        std::mem::take(&mut self.follow_up_tasks)
    }

    fn priority(&mut self) -> TaskPriority {
        TaskPriority::new(0, 0, TASK_PRIORITY_SAVE_BAND2, TASK_PRIORITY_BAND3_DEFAULT)
    }

    fn is_cancelled(&mut self) -> bool {
        false
    }

    fn debug_name(&self) -> &'static str {
        "SaveBlockData"
    }
}

#[cfg(test)]
mod tests {
    use super::SaveBlockDataTask;
    use crate::constants::voxel_constants::{
        TASK_PRIORITY_BAND3_DEFAULT, TASK_PRIORITY_SAVE_BAND2,
    };
    use crate::engine::StreamingDependency;
    use crate::math::Vector3i;
    use crate::storage::{ChannelId, VoxelBuffer};
    use crate::streams::{
        BlockDataOutputKind, LoadResult, MemoryStream, StreamResult, VoxelSaveQuery, VoxelStream,
        VoxelStreamError,
    };
    use crate::tasks::{
        AsyncDependencyTracker, TaskPriority, TaskRunOutcome, ThreadedTask, ThreadedTaskContext,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct FollowUpTask;

    impl ThreadedTask for FollowUpTask {
        fn run(self: Box<Self>, _ctx: ThreadedTaskContext) -> TaskRunOutcome {
            TaskRunOutcome::Complete(self)
        }
    }

    #[derive(Default)]
    struct CountingStream {
        saves: AtomicUsize,
        flushes: AtomicUsize,
    }

    impl CountingStream {
        fn saves(&self) -> usize {
            self.saves.load(Ordering::SeqCst)
        }

        fn flushes(&self) -> usize {
            self.flushes.load(Ordering::SeqCst)
        }
    }

    impl VoxelStream for CountingStream {
        fn save_voxel_block(&self, _query: VoxelSaveQuery<'_>) -> StreamResult<()> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn flush(&self) -> StreamResult<()> {
            self.flushes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct ErrorSaveStream;

    impl VoxelStream for ErrorSaveStream {
        fn save_voxel_block(&self, _query: VoxelSaveQuery<'_>) -> StreamResult<()> {
            Err(VoxelStreamError::Io("save failed".to_string()))
        }
    }

    fn filled_buffer(value: u64) -> VoxelBuffer {
        let mut voxels = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        voxels.set_voxel(value, 1, 0, 0, ChannelId::Type.index());
        voxels
    }

    #[test]
    fn run_saves_voxel_buffer_to_stream() {
        let stream = Arc::new(MemoryStream::new());
        let dependency = StreamingDependency::new(stream.clone());
        let position = Vector3i::new(3, 4, 5);
        let task = Box::new(SaveBlockDataTask::new_voxels(
            position,
            2,
            Some(filled_buffer(42)),
            dependency,
            None,
            false,
        ));

        let outcome = task.run(ThreadedTaskContext::new(0, TaskPriority::min()));

        assert!(matches!(outcome, TaskRunOutcome::Complete(_)));
        let mut loaded = VoxelBuffer::with_size(Vector3i::new(2, 2, 2));
        assert_eq!(
            stream.load_block(position, 2, &mut loaded),
            LoadResult::Found
        );
        assert_eq!(loaded.get_voxel(1, 0, 0, ChannelId::Type.index()), 42);
    }

    #[test]
    fn priority_matches_cpp_save_bands() {
        let stream = Arc::new(MemoryStream::new());
        let dependency = StreamingDependency::new(stream);
        let mut task = SaveBlockDataTask::new_voxels(
            Vector3i::default(),
            0,
            Some(filled_buffer(1)),
            dependency,
            None,
            false,
        );

        assert_eq!(
            task.priority(),
            TaskPriority::new(0, 0, TASK_PRIORITY_SAVE_BAND2, TASK_PRIORITY_BAND3_DEFAULT,)
        );
        assert!(!task.is_cancelled());
        assert_eq!(task.debug_name(), "SaveBlockData");
    }

    #[test]
    fn run_exposes_saved_block_output() {
        let stream = Arc::new(MemoryStream::new());
        let dependency = StreamingDependency::new(stream);
        let position = Vector3i::new(5, 6, 7);
        let mut task = SaveBlockDataTask::new_voxels(
            position,
            3,
            Some(filled_buffer(9)),
            dependency,
            None,
            false,
        );

        task.run_save();
        let output = task.take_output().unwrap();

        assert_eq!(output.kind, BlockDataOutputKind::Saved);
        assert_eq!(output.position_in_blocks, position);
        assert_eq!(output.lod_index, 3);
        assert!(output.had_voxels);
        assert!(output.voxels.is_none());
    }

    #[test]
    fn flushes_only_when_last_tracked_save_completes() {
        let stream = Arc::new(CountingStream::default());
        let dependency = StreamingDependency::new(stream.clone());
        let tracker = Arc::new(AsyncDependencyTracker::with_count(2));

        Box::new(SaveBlockDataTask::new_voxels(
            Vector3i::new(0, 0, 0),
            0,
            Some(filled_buffer(1)),
            dependency.clone(),
            Some(tracker.clone()),
            true,
        ))
        .run(ThreadedTaskContext::new(0, TaskPriority::min()));

        assert_eq!(stream.saves(), 1);
        assert_eq!(stream.flushes(), 0);
        assert_eq!(tracker.remaining_count(), 1);

        Box::new(SaveBlockDataTask::new_voxels(
            Vector3i::new(1, 0, 0),
            0,
            Some(filled_buffer(2)),
            dependency,
            Some(tracker.clone()),
            true,
        ))
        .run(ThreadedTaskContext::new(0, TaskPriority::min()));

        assert_eq!(stream.saves(), 2);
        assert_eq!(stream.flushes(), 1);
        assert!(tracker.is_complete());
    }

    #[test]
    fn missing_voxels_abort_tracker_without_saving() {
        let stream = Arc::new(CountingStream::default());
        let dependency = StreamingDependency::new(stream.clone());
        let tracker = Arc::new(AsyncDependencyTracker::with_count(1));
        let mut task = SaveBlockDataTask::new_voxels(
            Vector3i::default(),
            0,
            None,
            dependency,
            Some(tracker.clone()),
            true,
        );

        task.run_save();
        let output = task.take_output().unwrap();

        assert_eq!(output.kind, BlockDataOutputKind::Saved);
        assert!(output.dropped);
        assert!(!output.had_voxels);
        assert!(!task.has_run());
        assert!(tracker.is_aborted());
        assert_eq!(tracker.remaining_count(), 1);
        assert_eq!(stream.saves(), 0);
        assert_eq!(stream.flushes(), 0);
    }

    #[test]
    fn save_stream_error_emits_dropped_output_and_exposes_error() {
        let stream: Arc<dyn VoxelStream> = Arc::new(ErrorSaveStream);
        let dependency = StreamingDependency::new(stream);
        let mut task = SaveBlockDataTask::new_voxels(
            Vector3i::default(),
            0,
            Some(filled_buffer(1)),
            dependency,
            None,
            false,
        );

        task.run_save();
        let output = task.take_output().unwrap();

        assert_eq!(output.kind, BlockDataOutputKind::Saved);
        assert!(output.dropped);
        assert!(output.voxels.is_none());
        assert!(output.had_voxels);
        assert!(task.has_run());
        assert!(matches!(
            task.stream_error(),
            Some(VoxelStreamError::Io(message)) if message == "save failed"
        ));
    }

    #[test]
    fn completed_task_exposes_tracker_follow_up_tasks() {
        let stream = Arc::new(CountingStream::default());
        let dependency = StreamingDependency::new(stream);
        let tracker = Arc::new(AsyncDependencyTracker::with_count(1));
        tracker
            .set_next_tasks(vec![Box::new(FollowUpTask)])
            .unwrap();
        let task = Box::new(SaveBlockDataTask::new_voxels(
            Vector3i::default(),
            0,
            Some(filled_buffer(1)),
            dependency,
            Some(tracker),
            false,
        ));

        let TaskRunOutcome::Complete(mut completed) =
            task.run(ThreadedTaskContext::new(0, TaskPriority::min()))
        else {
            panic!("save task must complete");
        };

        assert_eq!(completed.take_follow_up_tasks().len(), 1);
    }
}
