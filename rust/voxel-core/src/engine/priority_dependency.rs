//! Streaming task priority helper ported from `engine/priority_dependency.*`.

use crate::constants::voxel_constants::{MAX_LOD, TASK_PRIORITY_BAND3_DEFAULT};
use crate::math::funcs::arithmetic_rshift;
use crate::math::vector3::math as v3;
use crate::math::Vector3f;
use crate::tasks::TaskPriority;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

const INITIAL_CLOSEST_DISTANCE_SQUARED: f32 = 99_999.0;

/// Shared viewer positions used by streaming tasks to re-evaluate priority.
pub struct PriorityViewersData {
    state: RwLock<PriorityViewersState>,
}

struct PriorityViewersState {
    viewers: Vec<Vector3f>,
    viewers_count: u32,
    highest_view_distance: f32,
}

impl PriorityViewersData {
    pub const DEFAULT_HIGHEST_VIEW_DISTANCE: f32 = 999_999.0;

    pub fn new(viewers: Vec<Vector3f>) -> Self {
        Self::with_highest_view_distance(viewers, Self::DEFAULT_HIGHEST_VIEW_DISTANCE)
    }

    pub fn with_highest_view_distance(viewers: Vec<Vector3f>, highest_view_distance: f32) -> Self {
        let viewers_count = viewers.len();
        assert!(
            viewers_count <= u32::MAX as usize,
            "viewer count does not fit in u32"
        );
        Self {
            state: RwLock::new(PriorityViewersState {
                viewers,
                viewers_count: viewers_count as u32,
                highest_view_distance,
            }),
        }
    }

    pub fn viewers(&self) -> Vec<Vector3f> {
        self.read_state().viewers.clone()
    }

    pub fn set_viewers(&self, viewers: Vec<Vector3f>) {
        let viewers_count = viewers.len();
        assert!(
            viewers_count <= u32::MAX as usize,
            "viewer count does not fit in u32"
        );
        let mut state = self.write_state();
        state.viewers = viewers;
        state.viewers_count = viewers_count as u32;
    }

    pub fn viewers_count(&self) -> u32 {
        self.read_state().viewers_count
    }

    pub fn set_viewers_count(&self, count: u32) {
        let mut state = self.write_state();
        assert!(
            count as usize <= state.viewers.len(),
            "viewer count cannot exceed viewer storage length"
        );
        state.viewers_count = count;
    }

    pub fn set_highest_view_distance(&self, highest_view_distance: f32) {
        self.write_state().highest_view_distance = highest_view_distance;
    }

    pub fn highest_view_distance(&self) -> f32 {
        self.read_state().highest_view_distance
    }

    fn read_state(&self) -> RwLockReadGuard<'_, PriorityViewersState> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_state(&self) -> RwLockWriteGuard<'_, PriorityViewersState> {
        self.state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriorityEvaluation {
    pub priority: TaskPriority,
    pub closest_distance_squared: f32,
}

/// Per-task priority dependency.
pub struct PriorityDependency {
    viewers: Arc<PriorityViewersData>,
    world_position: Vector3f,
    drop_distance_squared: f32,
}

impl PriorityDependency {
    pub fn new(
        viewers: Arc<PriorityViewersData>,
        world_position: Vector3f,
        drop_distance_squared: f32,
    ) -> Self {
        Self {
            viewers,
            world_position,
            drop_distance_squared,
        }
    }

    pub fn evaluate(&self, lod_index: u8, band2_priority: u8) -> PriorityEvaluation {
        assert!(
            usize::from(lod_index) < MAX_LOD,
            "LOD index {lod_index} is outside supported range 0..{MAX_LOD}"
        );
        let closest_distance_squared = self.closest_distance_squared();
        let distance = closest_distance_squared.sqrt() as i32;
        let falloff = arithmetic_rshift(distance, 4 + u32::from(lod_index));
        let band0 = (i32::from(TaskPriority::BAND_MAX) - falloff).max(0) as u8;
        let band1 = (MAX_LOD as i32 - i32::from(lod_index)) as u8;

        PriorityEvaluation {
            priority: TaskPriority::new(band0, band1, band2_priority, TASK_PRIORITY_BAND3_DEFAULT),
            closest_distance_squared,
        }
    }

    pub fn is_too_far(&self, closest_distance_squared: f32) -> bool {
        closest_distance_squared > self.drop_distance_squared
    }

    pub fn viewers(&self) -> &Arc<PriorityViewersData> {
        &self.viewers
    }

    pub const fn world_position(&self) -> Vector3f {
        self.world_position
    }

    pub const fn drop_distance_squared(&self) -> f32 {
        self.drop_distance_squared
    }

    fn closest_distance_squared(&self) -> f32 {
        let viewers = self.viewers.read_state();
        if viewers.viewers_count == 0 {
            return v3::length_squared(self.world_position);
        }

        viewers
            .viewers
            .iter()
            .take(viewers.viewers_count as usize)
            .map(|viewer| v3::distance_squared(*viewer, self.world_position))
            .fold(INITIAL_CLOSEST_DISTANCE_SQUARED, f32::min)
    }
}

#[cfg(test)]
mod tests {
    use super::{PriorityDependency, PriorityViewersData};
    use crate::constants::voxel_constants::{
        MAX_LOD, TASK_PRIORITY_BAND3_DEFAULT, TASK_PRIORITY_LOAD_BAND2,
    };
    use crate::math::Vector3f;
    use crate::tasks::TaskPriority;
    use std::sync::Arc;

    #[test]
    fn no_viewers_prioritizes_against_world_origin() {
        let viewers = Arc::new(PriorityViewersData::new(Vec::new()));
        let dependency =
            PriorityDependency::new(viewers, Vector3f::new(32.0, 0.0, 0.0), 1_000_000.0);

        let evaluation = dependency.evaluate(0, TASK_PRIORITY_LOAD_BAND2);

        assert_eq!(evaluation.closest_distance_squared, 1024.0);
        assert_eq!(
            evaluation.priority,
            TaskPriority::new(
                TaskPriority::BAND_MAX - 2,
                MAX_LOD as u8,
                TASK_PRIORITY_LOAD_BAND2,
                TASK_PRIORITY_BAND3_DEFAULT,
            )
        );
    }

    #[test]
    fn viewers_count_limits_active_viewers() {
        let viewers = Arc::new(PriorityViewersData::new(vec![
            Vector3f::new(100.0, 0.0, 0.0),
            Vector3f::new(0.0, 0.0, 0.0),
        ]));
        viewers.set_viewers_count(1);
        let dependency = PriorityDependency::new(viewers.clone(), Vector3f::default(), 10_000.0);

        let one_active = dependency.evaluate(0, TASK_PRIORITY_LOAD_BAND2);
        viewers.set_viewers_count(2);
        let two_active = dependency.evaluate(0, TASK_PRIORITY_LOAD_BAND2);

        assert_eq!(one_active.closest_distance_squared, 10_000.0);
        assert_eq!(two_active.closest_distance_squared, 0.0);
    }

    #[test]
    fn far_viewers_preserve_cpp_initial_closest_distance_cap() {
        let viewers = Arc::new(PriorityViewersData::new(vec![Vector3f::new(
            1_000_000.0,
            0.0,
            0.0,
        )]));
        let dependency = PriorityDependency::new(viewers, Vector3f::default(), 1_000_000_000.0);

        let evaluation = dependency.evaluate(0, TASK_PRIORITY_LOAD_BAND2);

        assert_eq!(evaluation.closest_distance_squared, 99_999.0);
    }

    #[test]
    fn viewer_updates_reprioritize_existing_dependency() {
        let viewers = Arc::new(PriorityViewersData::new(vec![Vector3f::new(
            100.0, 0.0, 0.0,
        )]));
        let dependency = PriorityDependency::new(viewers.clone(), Vector3f::default(), 10_000.0);

        assert_eq!(
            dependency
                .evaluate(0, TASK_PRIORITY_LOAD_BAND2)
                .closest_distance_squared,
            10_000.0
        );

        viewers.set_viewers(vec![Vector3f::default()]);

        assert_eq!(
            dependency
                .evaluate(0, TASK_PRIORITY_LOAD_BAND2)
                .closest_distance_squared,
            0.0
        );
    }

    #[test]
    #[should_panic(expected = "LOD index")]
    fn evaluate_rejects_lod_outside_engine_range() {
        let viewers = Arc::new(PriorityViewersData::new(Vec::new()));
        let dependency = PriorityDependency::new(viewers, Vector3f::default(), 1.0);

        dependency.evaluate(MAX_LOD as u8, TASK_PRIORITY_LOAD_BAND2);
    }

    #[test]
    fn too_far_uses_strict_distance_threshold() {
        let viewers = Arc::new(PriorityViewersData::new(Vec::new()));
        let dependency = PriorityDependency::new(viewers, Vector3f::new(5.0, 0.0, 0.0), 25.0);

        let evaluation = dependency.evaluate(0, TASK_PRIORITY_LOAD_BAND2);

        assert!(!dependency.is_too_far(evaluation.closest_distance_squared));
        assert!(PriorityDependency::new(
            dependency.viewers().clone(),
            Vector3f::new(5.0, 0.0, 0.0),
            24.9,
        )
        .is_too_far(evaluation.closest_distance_squared));
    }
}
