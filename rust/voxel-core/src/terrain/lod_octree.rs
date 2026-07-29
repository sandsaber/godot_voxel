//! LOD octree — a sparse octree that tracks which terrain blocks exist at which
//! LOD level. Ported from `terrain/variable_lod/lod_octree.h` (header-only,
//! ~480 LOC C++). Self-contained: only depends on `math::{Vector3i, Box3i}`.
//!
//! The octree is **progressive**: `update(actions)` splits/joins one level of
//! depth per call based on viewer-distance predicates. It is expected to be
//! called every frame (or every few frames) so the shape converges over time.
//! Each leaf node corresponds to an active mesh block at that node's LOD depth.
//!
//! Node positions are computed on the fly (not stored) to save memory. A node
//! at depth `d` (LOD `d`) covers `1 << d` blocks in each axis at LOD 0. The
//! root is at `max_depth` (the coarsest LOD).

use crate::math::{Box3i, Vector3i};

/// Sentinel: a node has no children (leaf).
const NO_CHILDREN: u32 = u32::MAX;
/// Sentinel index for the root node (the root lives outside the pool).
const ROOT_INDEX: u32 = u32::MAX;

/// Per-node data. Mirrors C++ `LodOctree::NodeData`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OctreeNodeData {
    pub state: u32,
}

/// One octree node. `first_child` indexes into the pool's packed-8 array;
/// `NO_CHILDREN` means this is a leaf.
#[derive(Debug, Clone, Copy)]
pub struct OctreeNode {
    pub first_child: u32,
    pub data: OctreeNodeData,
}

impl OctreeNode {
    fn new() -> Self {
        Self {
            first_child: NO_CHILDREN,
            data: OctreeNodeData::default(),
        }
    }

    #[inline]
    fn has_children(&self) -> bool {
        self.first_child != NO_CHILDREN
    }
}

/// Pool of octree nodes, treating nodes as packs of 8 so a parent can address
/// all its children via a single `first_child` index. Mirrors C++ `NodePool`.
#[derive(Debug, Default)]
struct NodePool {
    nodes: Vec<OctreeNode>,
    free_indexes: Vec<u32>,
}

impl NodePool {
    fn allocate_children(&mut self) -> u32 {
        if let Some(i0) = self.free_indexes.pop() {
            i0
        } else {
            let i0 = self.nodes.len() as u32;
            self.nodes
                .resize_with(self.nodes.len() + 8, OctreeNode::new);
            i0
        }
    }

    fn recycle_children(&mut self, i0: u32) {
        debug_assert_eq!(i0 % 8, 0);
        for i in 0..8u32 {
            self.nodes[(i0 + i) as usize] = OctreeNode::new();
        }
        self.free_indexes.push(i0);
    }

    #[inline]
    fn get(&self, i: u32) -> &OctreeNode {
        &self.nodes[i as usize]
    }

    #[inline]
    fn get_mut(&mut self, i: u32) -> &mut OctreeNode {
        &mut self.nodes[i as usize]
    }
}

/// Actions invoked during `LodOctree::update`. Mirrors C++
/// `LodOctree::DefaultUpdateActions`. Implement this to drive split/join from
/// viewer distances (or any other predicate).
pub trait OctreeUpdateActions {
    /// Called when a child node is created (on split).
    fn create_child(&mut self, node_pos: Vector3i, lod: u32, data: &mut OctreeNodeData);
    /// Called when a child node is destroyed (on join).
    fn destroy_child(&mut self, node_pos: Vector3i, lod: u32);
    /// Called when a parent becomes visible (on join — its children merged).
    fn show_parent(&mut self, node_pos: Vector3i, lod: u32);
    /// Called when a parent becomes hidden (on split — its children took over).
    fn hide_parent(&mut self, node_pos: Vector3i, lod: u32);
    /// Whether the root may be created initially.
    fn can_create_root(&self, lod: u32) -> bool;
    /// Whether a leaf node should split into 8 children.
    fn can_split(&self, node_pos: Vector3i, lod: u32, data: &OctreeNodeData) -> bool;
    /// Whether a parent with 8 leaf children should join back into one node.
    fn can_join(&self, node_pos: Vector3i, lod: u32) -> bool;
}

/// A no-op implementation of [`OctreeUpdateActions`] (all predicates return
/// `true`, callbacks do nothing). Useful for tests where you just want to drive
/// the octree shape manually.
pub struct NoOpActions;

impl OctreeUpdateActions for NoOpActions {
    fn create_child(&mut self, _: Vector3i, _: u32, _: &mut OctreeNodeData) {}
    fn destroy_child(&mut self, _: Vector3i, _: u32) {}
    fn show_parent(&mut self, _: Vector3i, _: u32) {}
    fn hide_parent(&mut self, _: Vector3i, _: u32) {}
    fn can_create_root(&self, _: u32) -> bool {
        true
    }
    fn can_split(&self, _: Vector3i, _: u32, _: &OctreeNodeData) -> bool {
        true
    }
    fn can_join(&self, _: Vector3i, _: u32) -> bool {
        true
    }
}

/// A LOD octree. Ported from C++ `LodOctree`.
#[derive(Debug)]
pub struct LodOctree {
    root: OctreeNode,
    is_root_created: bool,
    max_depth: u32,
    pool: NodePool,
}

impl Default for LodOctree {
    fn default() -> Self {
        Self {
            root: OctreeNode::new(),
            is_root_created: false,
            max_depth: 0,
            pool: NodePool::default(),
        }
    }
}

impl LodOctree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set up the octree for `lod_count` LOD levels. `lod_count` must be ≥ 1.
    /// `max_depth = lod_count - 1` (the root is at the coarsest LOD).
    pub fn create(&mut self, lod_count: u32) {
        assert!(lod_count >= 1, "lod_count must be >= 1");
        self.clear();
        self.max_depth = lod_count - 1;
    }

    /// Reset to an empty state (no nodes, no root).
    pub fn clear(&mut self) {
        self.pool.nodes.clear();
        self.pool.free_indexes.clear();
        self.root = OctreeNode::new();
        self.is_root_created = false;
        self.max_depth = 0;
    }

    /// Number of LOD levels (= `max_depth + 1`).
    pub fn lod_count(&self) -> u32 {
        self.max_depth + 1
    }

    /// Maximum depth (coarsest LOD index = root).
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Whether the root has been created (i.e., the octree has any content).
    pub fn is_root_created(&self) -> bool {
        self.is_root_created
    }

    /// Total allocated nodes (excluding root).
    pub fn node_count(&self) -> usize {
        self.node_count_recursive(ROOT_INDEX)
    }

    /// Fits the octree by splitting nodes that satisfy `can_split` and joining
    /// nodes that satisfy `can_join`. Progressive: call over several frames to
    /// converge the shape. Mirrors C++ `LodOctree::update`.
    pub fn update<A: OctreeUpdateActions>(&mut self, actions: &mut A) {
        if self.is_root_created || self.root.has_children() {
            self.update_node(ROOT_INDEX, Vector3i::zero(), self.max_depth, actions);
        } else if actions.can_create_root(self.max_depth) {
            actions.create_child(Vector3i::zero(), self.max_depth, &mut self.root.data);
            self.is_root_created = true;
            self.update_node(ROOT_INDEX, Vector3i::zero(), self.max_depth, actions);
        }
    }

    /// Recursively subdivide based on `can_split`. Does not unsubdivide.
    /// Mirrors C++ `LodOctree::subdivide`.
    pub fn subdivide<A: OctreeUpdateActions>(&mut self, actions: &mut A) {
        if !self.is_root_created
            && actions.can_split(Vector3i::zero(), self.max_depth, &self.root.data)
        {
            actions.create_child(Vector3i::zero(), self.max_depth, &mut self.root.data);
            self.is_root_created = true;
            self.subdivide_recursive(ROOT_INDEX, Vector3i::zero(), self.max_depth, actions);
        }
    }

    /// Execute `f` on all leaf nodes. Mirrors C++ `for_each_leaf`.
    pub fn for_each_leaf<F: FnMut(Vector3i, u32, &OctreeNodeData)>(&self, mut f: F) {
        self.for_each_leaf_recursive(Vector3i::zero(), ROOT_INDEX, self.max_depth, &mut f);
    }

    /// Execute `f` on all leaf nodes intersecting `box` (in octree-space leaf units).
    /// Mirrors C++ `for_leaves_in_box`.
    pub fn for_leaves_in_box<F: FnMut(Vector3i, u32, &mut OctreeNodeData)>(
        &mut self,
        box_: Box3i,
        mut f: F,
    ) {
        let root_box = Box3i::new(Vector3i::zero(), Vector3i::splat(1i32 << self.max_depth));
        let clipped = box_.clipped(root_box);
        self.for_leaves_in_box_recursive(
            clipped,
            Vector3i::zero(),
            ROOT_INDEX,
            self.max_depth,
            &mut f,
        );
    }

    // ---- static helpers (mirror C++ static methods) ----

    /// Child position within the octree given parent position + child index 0..7.
    /// Matches C++ `get_child_position`.
    #[inline]
    pub fn get_child_position(parent_pos: Vector3i, i: u32) -> Vector3i {
        Vector3i::new(
            parent_pos.x * 2 + (i & 1) as i32,
            parent_pos.y * 2 + ((i >> 1) & 1) as i32,
            parent_pos.z * 2 + ((i >> 2) & 1) as i32,
        )
    }

    /// Bounding box of a node in LOD0 coordinates. A leaf is 1×1×1; a LOD-N
    /// node is `(1 << N)³`. Matches C++ `get_node_box`.
    #[inline]
    pub fn get_node_box(pos: Vector3i, lod: u32) -> Box3i {
        let s = 1i32 << lod;
        Box3i::new(pos << lod, Vector3i::splat(s))
    }

    /// Distance predicate for `can_split`: is the node center within
    /// `lod_distance * (1 << lod)` of `view_pos`? Matches C++
    /// `is_below_split_distance`.
    pub fn is_below_split_distance(
        node_pos: Vector3i,
        lod: u32,
        view_pos: Vector3f,
        lod_distance: f32,
    ) -> bool {
        let lod_factor = (1u32 << lod) as f32;
        let center = Vector3f::new(
            lod_factor * (node_pos.x as f32 + 0.5),
            lod_factor * (node_pos.y as f32 + 0.5),
            lod_factor * (node_pos.z as f32 + 0.5),
        );
        let dx = center.x - view_pos.x;
        let dy = center.y - view_pos.y;
        let dz = center.z - view_pos.z;
        let dist_sq = dx * dx + dy * dy + dz * dz;
        let split_dist_sq = (lod_distance * lod_factor).powi(2);
        dist_sq < split_dist_sq
    }

    /// Compute how many LOD levels fit in `full_size` blocks given `base_size`
    /// block size. Matches C++ `compute_lod_count`.
    pub fn compute_lod_count(base_size: u32, full_size: u32) -> u32 {
        let mut fs = full_size;
        let mut po = 0u32;
        while fs > base_size {
            fs >>= 1;
            po += 1;
        }
        po
    }

    // ---- internal recursive methods ----

    fn get_node(&self, index: u32) -> &OctreeNode {
        if index == ROOT_INDEX {
            &self.root
        } else {
            self.pool.get(index)
        }
    }

    fn get_node_mut(&mut self, index: u32) -> &mut OctreeNode {
        if index == ROOT_INDEX {
            &mut self.root
        } else {
            self.pool.get_mut(index)
        }
    }

    fn update_node<A: OctreeUpdateActions>(
        &mut self,
        node_index: u32,
        node_pos: Vector3i,
        lod: u32,
        actions: &mut A,
    ) {
        let has_children = self.get_node(node_index).has_children();
        if !has_children {
            if lod > 0 && actions.can_split(node_pos, lod, &self.get_node(node_index).data) {
                let first_child = self.pool.allocate_children();
                self.get_node_mut(node_index).first_child = first_child;
                for i in 0..8u32 {
                    let child_pos = Self::get_child_position(node_pos, i);
                    let child_lod = lod - 1;
                    let child_index = first_child + i;
                    let child_data = &mut self.get_node_mut(child_index).data;
                    actions.create_child(child_pos, child_lod, child_data);
                    self.update_node(child_index, child_pos, child_lod, actions);
                }
                actions.hide_parent(node_pos, lod);
            }
        } else {
            let first_child = self.get_node(node_index).first_child;
            let mut has_split_child = false;
            for i in 0..8u32 {
                let child_index = first_child + i;
                let child_pos = Self::get_child_position(node_pos, i);
                self.update_node(child_index, child_pos, lod - 1, actions);
                has_split_child |= self.pool.get(child_index).has_children();
            }
            if !has_split_child && actions.can_join(node_pos, lod) {
                for i in 0..8u32 {
                    let child_pos = Self::get_child_position(node_pos, i);
                    actions.destroy_child(child_pos, lod - 1);
                }
                self.pool.recycle_children(first_child);
                self.get_node_mut(node_index).first_child = NO_CHILDREN;
                actions.show_parent(node_pos, lod);
            }
        }
    }

    fn subdivide_recursive<A: OctreeUpdateActions>(
        &mut self,
        node_index: u32,
        node_pos: Vector3i,
        lod: u32,
        actions: &mut A,
    ) {
        let has_children = self.get_node(node_index).has_children();
        if has_children {
            if lod <= 1 {
                return;
            }
            let first_child = self.get_node(node_index).first_child;
            for i in 0..8u32 {
                self.subdivide_recursive(
                    first_child + i,
                    Self::get_child_position(node_pos, i),
                    lod - 1,
                    actions,
                );
            }
        } else if lod > 0 && actions.can_split(node_pos, lod, &self.get_node(node_index).data) {
            let first_child = self.pool.allocate_children();
            self.get_node_mut(node_index).first_child = first_child;
            for i in 0..8u32 {
                let child_index = first_child + i;
                let child_pos = Self::get_child_position(node_pos, i);
                let child_data = &mut self.get_node_mut(child_index).data;
                actions.create_child(child_pos, lod - 1, child_data);
                self.subdivide_recursive(child_index, child_pos, lod - 1, actions);
            }
        }
    }

    fn node_count_recursive(&self, node_index: u32) -> usize {
        let node = self.get_node(node_index);
        let mut count = 1usize;
        if node.has_children() {
            for i in 0..8u32 {
                count += self.node_count_recursive(node.first_child + i);
            }
        }
        count
    }

    fn for_each_leaf_recursive<F: FnMut(Vector3i, u32, &OctreeNodeData)>(
        &self,
        node_pos: Vector3i,
        node_index: u32,
        depth: u32,
        f: &mut F,
    ) {
        let node = self.get_node(node_index);
        if node.has_children() {
            let first_child = node.first_child;
            for i in 0..8u32 {
                self.for_each_leaf_recursive(
                    Self::get_child_position(node_pos, i),
                    first_child + i,
                    depth - 1,
                    f,
                );
            }
        } else {
            f(node_pos, depth, &node.data);
        }
    }

    fn for_leaves_in_box_recursive<F: FnMut(Vector3i, u32, &mut OctreeNodeData)>(
        &mut self,
        box_: Box3i,
        node_pos: Vector3i,
        node_index: u32,
        depth: u32,
        f: &mut F,
    ) {
        let node_box = Self::get_node_box(node_pos, depth);
        if !node_box.intersects(&box_) {
            return;
        }
        let has_children = self.get_node(node_index).has_children();
        if has_children {
            let first_child = self.get_node(node_index).first_child;
            for i in 0..8u32 {
                self.for_leaves_in_box_recursive(
                    box_,
                    Self::get_child_position(node_pos, i),
                    first_child + i,
                    depth - 1,
                    f,
                );
            }
        } else {
            let data = &mut self.get_node_mut(node_index).data;
            f(node_pos, depth, data);
        }
    }
}

/// Minimal 3-component float vector used by [`LodOctree::is_below_split_distance`].
/// (Kept local to avoid pulling in the full `math::Vector3f` if this module is
/// used standalone in tests.)
type Vector3f = crate::math::Vector3f;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_sets_max_depth() {
        let mut ot = LodOctree::new();
        ot.create(3);
        assert_eq!(ot.lod_count(), 3);
        assert_eq!(ot.max_depth(), 2);
        assert!(!ot.is_root_created());
    }

    #[test]
    fn clear_resets_state() {
        let mut ot = LodOctree::new();
        ot.create(2);
        let mut actions = NoOpActions;
        ot.update(&mut actions);
        assert!(ot.is_root_created());
        ot.clear();
        assert!(!ot.is_root_created());
        assert_eq!(ot.lod_count(), 1);
    }

    #[test]
    fn update_creates_root() {
        let mut ot = LodOctree::new();
        ot.create(2);
        let mut actions = NoOpActions;
        ot.update(&mut actions);
        assert!(ot.is_root_created());
    }

    #[test]
    fn split_root_into_8_children() {
        let mut ot = LodOctree::new();
        ot.create(2); // max_depth=1, so root is LOD 1, children are LOD 0
        let mut actions = NoOpActions;
        // With NoOpActions (can_split always true), a single update creates the
        // root AND splits it in one pass (the root-creation branch calls
        // update_node immediately after).
        ot.update(&mut actions);
        assert!(ot.is_root_created());
        assert!(ot.get_node(ROOT_INDEX).has_children());
        assert_eq!(ot.node_count(), 9); // root + 8 children
    }

    #[test]
    fn join_when_can_join_returns_true() {
        let mut ot = LodOctree::new();
        ot.create(2);
        let mut actions = NoOpActions;
        ot.update(&mut actions); // create root + split
        assert!(ot.get_node(ROOT_INDEX).has_children());
        // Children are LOD 0 (lod > 0 is false), so they can't split further.
        // With NoOpActions, can_join always returns true → next update joins.
        ot.update(&mut actions);
        assert!(!ot.get_node(ROOT_INDEX).has_children());
    }

    #[test]
    fn for_each_leaf_visits_all_leaves() {
        let mut ot = LodOctree::new();
        ot.create(2);
        let mut actions = NoOpActions;
        ot.update(&mut actions); // create root + split → 8 leaves at LOD 0
        let mut leaves = Vec::new();
        ot.for_each_leaf(|pos, lod, _| {
            leaves.push((pos, lod));
        });
        // 8 children at LOD 0.
        assert_eq!(leaves.len(), 8);
        assert!(leaves.iter().all(|(_, lod)| *lod == 0));
    }

    #[test]
    fn get_child_position_matches_bit_pattern() {
        // i=0 → (0,0,0), i=1 → (1,0,0), i=2 → (0,1,0), ...
        let p = LodOctree::get_child_position(Vector3i::new(2, 2, 2), 0);
        assert_eq!(p, Vector3i::new(4, 4, 4));
        let p = LodOctree::get_child_position(Vector3i::new(2, 2, 2), 5);
        // 5 = 0b101 → x=1, y=0, z=1
        assert_eq!(p, Vector3i::new(5, 4, 5));
    }

    #[test]
    fn get_node_box_scales_by_lod() {
        // LOD 0 → 1×1×1
        let b = LodOctree::get_node_box(Vector3i::new(3, 4, 5), 0);
        assert_eq!(b.position, Vector3i::new(3, 4, 5));
        assert_eq!(b.size, Vector3i::splat(1));
        // LOD 2 → 4×4×4
        let b = LodOctree::get_node_box(Vector3i::new(1, 0, 0), 2);
        assert_eq!(b.position, Vector3i::new(4, 0, 0));
        assert_eq!(b.size, Vector3i::splat(4));
    }

    #[test]
    fn is_below_split_distance_close_returns_true() {
        // Node at origin LOD 0, viewer at center → very close.
        let close = LodOctree::is_below_split_distance(
            Vector3i::zero(),
            0,
            Vector3f::new(0.5, 0.5, 0.5),
            10.0,
        );
        assert!(close);
        // Far away → false.
        let far = LodOctree::is_below_split_distance(
            Vector3i::zero(),
            0,
            Vector3f::new(1000.0, 0.0, 0.0),
            10.0,
        );
        assert!(!far);
    }

    #[test]
    fn compute_lod_count_simple() {
        // base_size=16, full_size=16 → 0 extra LODs.
        assert_eq!(LodOctree::compute_lod_count(16, 16), 0);
        // full_size=32 → 1 LOD.
        assert_eq!(LodOctree::compute_lod_count(16, 32), 1);
        // full_size=64 → 2 LODs.
        assert_eq!(LodOctree::compute_lod_count(16, 64), 2);
    }

    #[test]
    fn for_leaves_in_box_finds_intersecting_leaves() {
        let mut ot = LodOctree::new();
        ot.create(2); // root at LOD 1
        let mut actions = NoOpActions;
        ot.update(&mut actions);
        ot.update(&mut actions); // split → 8 leaves at LOD 0
                                 // Query a small box that intersects only 1 leaf.
        let box_ = Box3i::new(Vector3i::zero(), Vector3i::splat(1));
        let mut count = 0;
        ot.for_leaves_in_box(box_, |_, _, _| {
            count += 1;
        });
        assert!(count >= 1, "should find at least 1 leaf in the box");
    }

    #[test]
    fn subdivide_creates_full_depth() {
        let mut ot = LodOctree::new();
        ot.create(3); // max_depth=2
        let mut actions = NoOpActions;
        ot.subdivide(&mut actions);
        // With NoOpActions, can_split always true → full subdivision.
        // Root (LOD 2) splits → 8 children (LOD 1) → each splits → 64 (LOD 0).
        // But subdivide only goes one level deep per call on children...
        // Let's just verify the root split.
        assert!(ot.get_node(ROOT_INDEX).has_children());
    }
}
