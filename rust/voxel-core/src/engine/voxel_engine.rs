//! Engine-level registry and shared viewer priority data.
//!
//! This is the engine-agnostic subset of `engine/voxel_engine.*`: volume and
//! viewer registries plus `sync_viewers_task_priority_data`.

use super::PriorityViewersData;
use crate::math::Vector3f;
use std::marker::PhantomData;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SlotKey {
    index: u32,
    generation: u32,
}

trait SlotHandle: Copy {
    fn from_key(key: SlotKey) -> Self;
    fn key(self) -> SlotKey;
}

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

#[derive(Debug)]
struct GenerationalSlotMap<T, H> {
    slots: Vec<Slot<T>>,
    free_indices: Vec<usize>,
    _marker: PhantomData<fn() -> H>,
}

impl<T, H> Default for GenerationalSlotMap<T, H> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free_indices: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<T, H: SlotHandle> GenerationalSlotMap<T, H> {
    fn add(&mut self, value: T) -> H {
        if let Some(index) = self.free_indices.pop() {
            let slot = &mut self.slots[index];
            debug_assert!(slot.value.is_none());
            slot.value = Some(value);
            return H::from_key(SlotKey {
                index: index as u32,
                generation: slot.generation,
            });
        }

        let index = self.slots.len();
        assert!(
            u32::try_from(index).is_ok(),
            "slot index does not fit in u32"
        );
        self.slots.push(Slot {
            generation: 1,
            value: Some(value),
        });
        H::from_key(SlotKey {
            index: index as u32,
            generation: 1,
        })
    }

    fn remove(&mut self, handle: H) -> bool {
        let key = handle.key();
        let Some(slot) = self.slots.get_mut(key.index as usize) else {
            return false;
        };
        if slot.generation != key.generation || slot.value.is_none() {
            return false;
        }

        slot.value = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free_indices.push(key.index as usize);
        true
    }

    fn exists(&self, handle: H) -> bool {
        self.get(handle).is_some()
    }

    fn get(&self, handle: H) -> Option<&T> {
        let key = handle.key();
        self.slots.get(key.index as usize).and_then(|slot| {
            if slot.generation == key.generation {
                slot.value.as_ref()
            } else {
                None
            }
        })
    }

    fn get_mut(&mut self, handle: H) -> Option<&mut T> {
        let key = handle.key();
        self.slots.get_mut(key.index as usize).and_then(|slot| {
            if slot.generation == key.generation {
                slot.value.as_mut()
            } else {
                None
            }
        })
    }

    fn count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
    }

    fn values(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|slot| slot.value.as_ref())
    }
}

/// Generational volume handle, equivalent to C++ `VolumeID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VolumeId {
    index: u32,
    generation: u32,
}

impl SlotHandle for VolumeId {
    fn from_key(key: SlotKey) -> Self {
        Self {
            index: key.index,
            generation: key.generation,
        }
    }

    fn key(self) -> SlotKey {
        SlotKey {
            index: self.index,
            generation: self.generation,
        }
    }
}

/// Generational viewer handle, equivalent to C++ `ViewerID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewerId {
    index: u32,
    generation: u32,
}

impl SlotHandle for ViewerId {
    fn from_key(key: SlotKey) -> Self {
        Self {
            index: key.index,
            generation: key.generation,
        }
    }

    fn key(self) -> SlotKey {
        SlotKey {
            index: self.index,
            generation: self.generation,
        }
    }
}

/// Per-viewer horizontal/vertical view distances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerDistances {
    pub horizontal: u32,
    pub vertical: u32,
}

impl ViewerDistances {
    pub const fn max(self) -> u32 {
        if self.horizontal > self.vertical {
            self.horizontal
        } else {
            self.vertical
        }
    }
}

impl Default for ViewerDistances {
    fn default() -> Self {
        Self {
            horizontal: 128,
            vertical: 128,
        }
    }
}

#[derive(Debug, Default)]
struct Volume;

/// A viewer tracked by [`VoxelEngine`].
#[derive(Debug, Clone, PartialEq)]
pub struct Viewer {
    pub world_position: Vector3f,
    pub view_distances: ViewerDistances,
    pub require_collisions: bool,
    pub require_visuals: bool,
    pub requires_data_block_notifications: bool,
    pub network_peer_id: i32,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            world_position: Vector3f::zero(),
            view_distances: ViewerDistances::default(),
            require_collisions: true,
            require_visuals: true,
            requires_data_block_notifications: false,
            network_peer_id: -1,
        }
    }
}

/// Engine-agnostic subset of C++ `VoxelEngine`.
pub struct VoxelEngine {
    volumes: GenerationalSlotMap<Volume, VolumeId>,
    viewers: GenerationalSlotMap<Viewer, ViewerId>,
    shared_priority_dependency: Arc<PriorityViewersData>,
}

impl Default for VoxelEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl VoxelEngine {
    pub fn new() -> Self {
        Self {
            volumes: GenerationalSlotMap::default(),
            viewers: GenerationalSlotMap::default(),
            shared_priority_dependency: Arc::new(PriorityViewersData::new(Vec::new())),
        }
    }

    pub fn add_volume(&mut self) -> VolumeId {
        self.volumes.add(Volume)
    }

    pub fn remove_volume(&mut self, volume_id: VolumeId) -> bool {
        self.volumes.remove(volume_id)
    }

    pub fn is_volume_valid(&self, volume_id: VolumeId) -> bool {
        self.volumes.exists(volume_id)
    }

    pub fn volume_count(&self) -> usize {
        self.volumes.count()
    }

    pub fn add_viewer(&mut self) -> ViewerId {
        self.viewers.add(Viewer::default())
    }

    pub fn remove_viewer(&mut self, viewer_id: ViewerId) -> bool {
        self.viewers.remove(viewer_id)
    }

    pub fn viewer_exists(&self, viewer_id: ViewerId) -> bool {
        self.viewers.exists(viewer_id)
    }

    pub fn viewer_count(&self) -> usize {
        self.viewers.count()
    }

    pub fn set_viewer_position(&mut self, viewer_id: ViewerId, position: Vector3f) -> bool {
        let Some(viewer) = self.viewers.get_mut(viewer_id) else {
            return false;
        };
        viewer.world_position = position;
        true
    }

    pub fn viewer_position(&self, viewer_id: ViewerId) -> Option<Vector3f> {
        self.viewers
            .get(viewer_id)
            .map(|viewer| viewer.world_position)
    }

    pub fn set_viewer_distances(
        &mut self,
        viewer_id: ViewerId,
        distances: ViewerDistances,
    ) -> bool {
        let Some(viewer) = self.viewers.get_mut(viewer_id) else {
            return false;
        };
        viewer.view_distances = distances;
        true
    }

    pub fn viewer_distances(&self, viewer_id: ViewerId) -> Option<ViewerDistances> {
        self.viewers
            .get(viewer_id)
            .map(|viewer| viewer.view_distances)
    }

    pub fn set_viewer_requires_visuals(&mut self, viewer_id: ViewerId, enabled: bool) -> bool {
        let Some(viewer) = self.viewers.get_mut(viewer_id) else {
            return false;
        };
        viewer.require_visuals = enabled;
        true
    }

    pub fn viewer_requires_visuals(&self, viewer_id: ViewerId) -> Option<bool> {
        self.viewers
            .get(viewer_id)
            .map(|viewer| viewer.require_visuals)
    }

    pub fn set_viewer_requires_collisions(&mut self, viewer_id: ViewerId, enabled: bool) -> bool {
        let Some(viewer) = self.viewers.get_mut(viewer_id) else {
            return false;
        };
        viewer.require_collisions = enabled;
        true
    }

    pub fn viewer_requires_collisions(&self, viewer_id: ViewerId) -> Option<bool> {
        self.viewers
            .get(viewer_id)
            .map(|viewer| viewer.require_collisions)
    }

    pub fn set_viewer_requires_data_block_notifications(
        &mut self,
        viewer_id: ViewerId,
        enabled: bool,
    ) -> bool {
        let Some(viewer) = self.viewers.get_mut(viewer_id) else {
            return false;
        };
        viewer.requires_data_block_notifications = enabled;
        true
    }

    pub fn viewer_requires_data_block_notifications(&self, viewer_id: ViewerId) -> Option<bool> {
        self.viewers
            .get(viewer_id)
            .map(|viewer| viewer.requires_data_block_notifications)
    }

    pub fn set_viewer_network_peer_id(&mut self, viewer_id: ViewerId, peer_id: i32) -> bool {
        let Some(viewer) = self.viewers.get_mut(viewer_id) else {
            return false;
        };
        viewer.network_peer_id = peer_id;
        true
    }

    pub fn viewer_network_peer_id(&self, viewer_id: ViewerId) -> Option<i32> {
        self.viewers
            .get(viewer_id)
            .map(|viewer| viewer.network_peer_id)
    }

    pub fn shared_viewers_data(&self) -> Arc<PriorityViewersData> {
        self.shared_priority_dependency.clone()
    }

    pub fn sync_viewers_task_priority_data(&self) {
        let mut max_distance = 0u32;
        let viewers: Vec<Vector3f> = self
            .viewers
            .values()
            .map(|viewer| {
                max_distance = max_distance.max(viewer.view_distances.max());
                viewer.world_position
            })
            .collect();

        self.shared_priority_dependency.set_viewers(viewers);
        self.shared_priority_dependency
            .set_highest_view_distance((max_distance as f32) * 2.0);
    }

    pub fn process(&self) {
        self.sync_viewers_task_priority_data();
    }
}

#[cfg(test)]
mod tests {
    use super::{ViewerDistances, VoxelEngine};
    use crate::math::Vector3f;

    #[test]
    fn viewer_ids_are_generational_after_remove_and_reuse() {
        let mut engine = VoxelEngine::new();
        let first = engine.add_viewer();
        assert!(engine.viewer_exists(first));

        assert!(engine.remove_viewer(first));
        assert!(!engine.viewer_exists(first));

        let second = engine.add_viewer();
        assert_ne!(first, second);
        assert!(!engine.set_viewer_position(first, Vector3f::new(1.0, 2.0, 3.0)));
        assert!(engine.set_viewer_position(second, Vector3f::new(4.0, 5.0, 6.0)));
    }

    #[test]
    fn volume_ids_are_generational_after_remove_and_reuse() {
        let mut engine = VoxelEngine::new();
        let first = engine.add_volume();
        assert!(engine.is_volume_valid(first));

        assert!(engine.remove_volume(first));
        assert!(!engine.is_volume_valid(first));

        let second = engine.add_volume();
        assert_ne!(first, second);
        assert!(!engine.remove_volume(first));
        assert!(engine.is_volume_valid(second));
    }

    #[test]
    fn viewer_properties_round_trip() {
        let mut engine = VoxelEngine::new();
        let viewer = engine.add_viewer();

        assert!(engine.set_viewer_position(viewer, Vector3f::new(1.0, 2.0, 3.0)));
        assert!(engine.set_viewer_distances(
            viewer,
            ViewerDistances {
                horizontal: 64,
                vertical: 96,
            },
        ));
        assert!(engine.set_viewer_requires_visuals(viewer, false));
        assert!(engine.set_viewer_requires_collisions(viewer, false));
        assert!(engine.set_viewer_requires_data_block_notifications(viewer, true));
        assert!(engine.set_viewer_network_peer_id(viewer, 42));

        assert_eq!(
            engine.viewer_position(viewer),
            Some(Vector3f::new(1.0, 2.0, 3.0))
        );
        assert_eq!(
            engine.viewer_distances(viewer),
            Some(ViewerDistances {
                horizontal: 64,
                vertical: 96,
            })
        );
        assert_eq!(engine.viewer_requires_visuals(viewer), Some(false));
        assert_eq!(engine.viewer_requires_collisions(viewer), Some(false));
        assert_eq!(
            engine.viewer_requires_data_block_notifications(viewer),
            Some(true)
        );
        assert_eq!(engine.viewer_network_peer_id(viewer), Some(42));
    }

    #[test]
    fn sync_viewers_task_priority_data_exports_positions_and_cancel_distance() {
        let mut engine = VoxelEngine::new();
        let first = engine.add_viewer();
        let second = engine.add_viewer();
        assert!(engine.set_viewer_position(first, Vector3f::new(10.0, 0.0, 0.0)));
        assert!(engine.set_viewer_position(second, Vector3f::new(20.0, 0.0, 0.0)));
        assert!(engine.set_viewer_distances(
            first,
            ViewerDistances {
                horizontal: 32,
                vertical: 48,
            },
        ));
        assert!(engine.set_viewer_distances(
            second,
            ViewerDistances {
                horizontal: 80,
                vertical: 16,
            },
        ));

        engine.sync_viewers_task_priority_data();

        let shared = engine.shared_viewers_data();
        assert_eq!(shared.viewers_count(), 2);
        assert_eq!(
            shared.viewers(),
            vec![Vector3f::new(10.0, 0.0, 0.0), Vector3f::new(20.0, 0.0, 0.0)]
        );
        assert_eq!(shared.highest_view_distance(), 160.0);
    }

    #[test]
    fn sync_viewers_handles_extreme_view_distance_without_u32_overflow() {
        let mut engine = VoxelEngine::new();
        let viewer = engine.add_viewer();
        assert!(engine.set_viewer_distances(
            viewer,
            ViewerDistances {
                horizontal: u32::MAX,
                vertical: 0,
            },
        ));

        engine.sync_viewers_task_priority_data();

        let shared = engine.shared_viewers_data();
        assert_eq!(shared.highest_view_distance(), (u32::MAX as f32) * 2.0);
    }

    #[test]
    fn process_syncs_shared_viewer_priority_data() {
        let mut engine = VoxelEngine::new();
        let viewer = engine.add_viewer();
        assert!(engine.set_viewer_position(viewer, Vector3f::new(7.0, 8.0, 9.0)));

        engine.process();

        let shared = engine.shared_viewers_data();
        assert_eq!(shared.viewers_count(), 1);
        assert_eq!(shared.viewers(), vec![Vector3f::new(7.0, 8.0, 9.0)]);
    }
}
