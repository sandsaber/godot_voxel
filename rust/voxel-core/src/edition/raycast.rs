//! DDA voxel raycasting — Amanatides & Woo fast voxel traversal.
//!
//! Ports `util/voxel_raycast.h` (header-only, ~130 LOC C++). The algorithm
//! steps through a uniform voxel grid along a ray, calling a predicate at
//! each voxel to determine if it's a hit.
//!
//! Reference: Amanatides & Woo, "A Fast Voxel Traversal Algorithm for
//! Ray Tracing" — <cse.yorku.ca/~amana/research/grid.pdf>.

use crate::math::{Vector3f, Vector3i};

/// State snapshot passed to the raycast predicate.
#[derive(Debug, Clone, Copy)]
pub struct VoxelRaycastState {
    /// The voxel the ray just left.
    pub prev_position: Vector3i,
    /// Parametric distance at `prev_position`.
    pub prev_distance: f32,
    /// The voxel the ray just entered.
    pub position: Vector3i,
    /// Parametric distance at `position`.
    pub distance: f32,
}

/// Result of a successful voxel raycast.
#[derive(Debug, Clone, Copy)]
pub struct VoxelRaycastHit {
    pub position: Vector3i,
    pub previous_position: Vector3i,
    pub distance: f32,
    /// Face normal derived from the step direction (points back toward the ray origin).
    pub normal: Vector3i,
}

/// DDA voxel raycast. Traverses voxels from `origin` along `direction`
/// (must be normalized), up to `max_distance`. Calls `predicate` at each
/// voxel; returns `Some(hit)` on the first voxel where `predicate` returns
/// `true`, or `None` if the ray exits `max_distance` without a hit.
///
/// Matches C++ `voxel_raycast` (util/voxel_raycast.h).
pub fn voxel_raycast<P>(
    origin: Vector3f,
    direction: Vector3f,
    max_distance: f32,
    predicate: P,
) -> Option<VoxelRaycastHit>
where
    P: FnMut(&VoxelRaycastState) -> bool,
{
    let mut predicate = predicate;

    // Reject NaN directions.
    if direction.x.is_nan() || direction.y.is_nan() || direction.z.is_nan() {
        return None;
    }

    let mut hit_pos = Vector3i::new(
        origin.x.floor() as i32,
        origin.y.floor() as i32,
        origin.z.floor() as i32,
    );

    // Per-axis integer step direction.
    let step = Vector3i::new(
        if direction.x > 0.0 {
            1
        } else if direction.x < 0.0 {
            -1
        } else {
            0
        },
        if direction.y > 0.0 {
            1
        } else if direction.y < 0.0 {
            -1
        } else {
            0
        },
        if direction.z > 0.0 {
            1
        } else if direction.z < 0.0 {
            -1
        } else {
            0
        },
    );

    // Per-axis t-delta (distance to cross one full voxel boundary).
    let large = 1.0e30f32;
    let tdelta = Vector3f::new(
        if direction.x != 0.0 {
            1.0 / direction.x.abs()
        } else {
            large
        },
        if direction.y != 0.0 {
            1.0 / direction.y.abs()
        } else {
            large
        },
        if direction.z != 0.0 {
            1.0 / direction.z.abs()
        } else {
            large
        },
    );

    // Per-axis t-cross (distance to the next grid boundary from origin).
    let mut tcross = [0.0f32; 3];
    // X
    if step.x > 0 {
        tcross[0] = (origin.x.floor() + 1.0 - origin.x) * tdelta.x;
    } else if step.x < 0 {
        tcross[0] = (origin.x - origin.x.floor()) * tdelta.x;
    } else {
        tcross[0] = large;
    }
    // Y
    if step.y > 0 {
        tcross[1] = (origin.y.floor() + 1.0 - origin.y) * tdelta.y;
    } else if step.y < 0 {
        tcross[1] = (origin.y - origin.y.floor()) * tdelta.y;
    } else {
        tcross[1] = large;
    }
    // Z
    if step.z > 0 {
        tcross[2] = (origin.z.floor() + 1.0 - origin.z) * tdelta.z;
    } else if step.z < 0 {
        tcross[2] = (origin.z - origin.z.floor()) * tdelta.z;
    } else {
        tcross[2] = large;
    }

    // Edge case: if origin sits exactly on a boundary, tcross can be 0,
    // which would immediately advance past it. Add one tdelta.
    for i in 0..3 {
        if tcross[i] == 0.0 {
            tcross[i] += tdelta[i];
            // For negative direction, the first voxel is behind us.
            if step[i] < 0 {
                hit_pos[i] -= 1;
            }
        }
    }

    let mut t = 0.0f32;
    let mut normal;
    let _ = Vector3i::zero(); // suppress unused warning if Vector3i::zero is never used after this

    loop {
        let prev_pos = hit_pos;
        let t_prev = t;

        // Advance the axis with the smallest tcross.
        if tcross[0] < tcross[1] {
            if tcross[0] < tcross[2] {
                // X axis advances.
                if tcross[0] > max_distance {
                    return None;
                }
                hit_pos.x += step.x;
                t = tcross[0];
                tcross[0] += tdelta.x;
                normal = Vector3i::new(-step.x, 0, 0);
            } else {
                // Z axis advances.
                if tcross[2] > max_distance {
                    return None;
                }
                hit_pos.z += step.z;
                t = tcross[2];
                tcross[2] += tdelta.z;
                normal = Vector3i::new(0, 0, -step.z);
            }
        } else {
            if tcross[1] < tcross[2] {
                // Y axis advances.
                if tcross[1] > max_distance {
                    return None;
                }
                hit_pos.y += step.y;
                t = tcross[1];
                tcross[1] += tdelta.y;
                normal = Vector3i::new(0, -step.y, 0);
            } else {
                // Z axis advances.
                if tcross[2] > max_distance {
                    return None;
                }
                hit_pos.z += step.z;
                t = tcross[2];
                tcross[2] += tdelta.z;
                normal = Vector3i::new(0, 0, -step.z);
            }
        }

        let state = VoxelRaycastState {
            prev_position: prev_pos,
            prev_distance: t_prev,
            position: hit_pos,
            distance: t,
        };

        if predicate(&state) {
            return Some(VoxelRaycastHit {
                position: hit_pos,
                previous_position: prev_pos,
                distance: t,
                normal,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raycast_hits_solid_voxel_along_x() {
        // Ray from (0.5, 0.5, 0.5) along +X. Voxels 1,0,0 should be solid.
        let hit = voxel_raycast(
            Vector3f::new(0.5, 0.5, 0.5),
            Vector3f::new(1.0, 0.0, 0.0),
            100.0,
            |s| s.position == Vector3i::new(3, 0, 0),
        );
        assert!(hit.is_some(), "should hit voxel (3,0,0)");
        let h = hit.unwrap();
        assert_eq!(h.position, Vector3i::new(3, 0, 0));
        assert_eq!(h.previous_position, Vector3i::new(2, 0, 0));
        assert_eq!(h.normal, Vector3i::new(-1, 0, 0));
    }

    #[test]
    fn raycast_returns_none_when_no_hit() {
        let hit = voxel_raycast(
            Vector3f::new(0.5, 0.5, 0.5),
            Vector3f::new(1.0, 0.0, 0.0),
            5.0,
            |_| false, // No solid voxels
        );
        assert!(hit.is_none(), "should return None");
    }

    #[test]
    fn raycast_traverses_diagonal() {
        // Ray from origin along (1,1,1)/sqrt(3). Should hit voxel (2,2,2).
        let dir = Vector3f::new(1.0, 1.0, 1.0);
        let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
        let ndir = Vector3f::new(dir.x / len, dir.y / len, dir.z / len);

        let hit = voxel_raycast(Vector3f::new(0.5, 0.5, 0.5), ndir, 100.0, |s| {
            s.position.x >= 2 && s.position.y >= 2 && s.position.z >= 2
        });
        assert!(hit.is_some(), "should hit diagonal voxel");
        let h = hit.unwrap();
        assert!(h.position.x >= 2 && h.position.y >= 2 && h.position.z >= 2);
    }

    #[test]
    fn raycast_negative_direction() {
        // Ray from (5.5, 0.5, 0.5) along -X. Should hit voxel (1,0,0).
        let hit = voxel_raycast(
            Vector3f::new(5.5, 0.5, 0.5),
            Vector3f::new(-1.0, 0.0, 0.0),
            100.0,
            |s| s.position == Vector3i::new(1, 0, 0),
        );
        assert!(hit.is_some());
        let h = hit.unwrap();
        assert_eq!(h.position, Vector3i::new(1, 0, 0));
        assert_eq!(h.normal, Vector3i::new(1, 0, 0)); // normal points +X (back toward origin)
    }
}
