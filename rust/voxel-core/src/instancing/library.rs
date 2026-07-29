//! Instance library — collection of scatter item definitions.
//!
//! Ports `terrain/instancing/voxel_instance_library.{h,cpp}` (engine-agnostic half).

/// Type of mesh used for an instance item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceMeshType {
    /// Use a MultiMesh3D with a fixed mesh.
    MultiMesh,
    /// Spawn a scene instance.
    Scene,
}

/// One entry in an [`InstanceLibrary`]. Defines what to scatter and how.
#[derive(Debug, Clone)]
pub struct InstanceLibraryItem {
    /// Name for editor display / debugging.
    pub name: String,
    /// Mesh type for rendering.
    pub mesh_type: InstanceMeshType,
    /// Density multiplier (instances per unit surface area).
    pub density: f32,
    /// Minimum scale factor.
    pub min_scale: f32,
    /// Maximum scale factor.
    pub max_scale: f32,
    /// Whether to snap instances to the ground normal.
    pub snap_to_normal: bool,
    /// Random yaw range in radians [min, max].
    pub yaw_range: (f32, f32),
}

impl Default for InstanceLibraryItem {
    fn default() -> Self {
        Self {
            name: String::new(),
            mesh_type: InstanceMeshType::MultiMesh,
            density: 0.1,
            min_scale: 0.8,
            max_scale: 1.2,
            snap_to_normal: true,
            yaw_range: (0.0, std::f32::consts::TAU),
        }
    }
}

/// A library of scatter items. Each item corresponds to one surface layer
/// (e.g. trees, rocks, grass). The instancer generates instances per item
/// based on terrain surface data.
#[derive(Debug, Clone, Default)]
pub struct InstanceLibrary {
    pub items: Vec<InstanceLibraryItem>,
}

impl InstanceLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_item(&mut self, item: InstanceLibraryItem) -> usize {
        let index = self.items.len();
        self.items.push(item);
        index
    }

    pub fn get_item(&self, index: usize) -> Option<&InstanceLibraryItem> {
        self.items.get(index)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_add_and_get() {
        let mut lib = InstanceLibrary::new();
        let idx = lib.add_item(InstanceLibraryItem {
            name: "Trees".into(),
            density: 0.05,
            ..Default::default()
        });
        assert_eq!(idx, 0);
        assert_eq!(lib.len(), 1);
        assert_eq!(lib.get_item(0).unwrap().name, "Trees");
    }

    #[test]
    fn default_item_has_sensible_values() {
        let item = InstanceLibraryItem::default();
        assert!(item.density > 0.0);
        assert!(item.min_scale <= item.max_scale);
    }
}
