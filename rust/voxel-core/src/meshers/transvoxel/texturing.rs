//! Texturing modes for the transvoxel mesher.
//!
//! Ports `transvoxel_materials_null.h`, `transvoxel_materials_single_s4.h`,
//! and the `TexturingMode` enum from `transvoxel.h`. The material processor
//! selects up to 4 material indices per cell for blending in the shader.
//!
//! Currently implements:
//! - `TexturingMode::None` — no texture data (existing behavior).
//! - `TexturingMode::SingleS4` — one 8-bit material index per voxel, up to 4
//!   blend in shader. Selects the 4 most-represented materials in each cell.

/// How textures are assigned to transvoxel vertices. Matches C++
/// `transvoxel::TexturingMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TexturingMode {
    /// No texture data. Vertices carry no material index.
    #[default]
    None,
    /// Blends the 4 most-represented textures in the given block. (Not yet
    /// implemented — deferred.)
    Mixel4S4,
    /// Each voxel has one 8-bit material index; up to 4 blend in shader.
    SingleS4,
    /// Each voxel has one 8-bit material index; up to 2 blend in shader.
    SingleS2,
}

/// A weighted material index used during cell material selection.
#[derive(Debug, Clone, Copy, Default)]
struct WeightedIndex {
    index: u8,
    weight: u32,
}

/// The result of material selection for one regular cell (2³ voxels).
#[derive(Debug, Clone, Copy, Default)]
pub struct CellMaterials {
    /// The 4 selected material indices (most-represented first).
    pub selected_indices: [u8; 4],
    /// Packed 4-byte representation of selected_indices for shader use.
    pub packed_indices: u32,
    /// Which of the 4 selected materials each corner uses (0-3).
    pub component_indices: [u8; 8],
}

/// The result of material selection for one transition cell (2×3×3 corners).
#[derive(Debug, Clone, Copy, Default)]
pub struct TransitionCellMaterials {
    pub selected_indices: [u8; 4],
    pub packed_indices: u32,
    pub component_indices: [u8; 9],
}

/// Packs 4 bytes into a u32 (little-endian).
pub fn pack_bytes(a: [u8; 4]) -> u32 {
    (a[0] as u32) | ((a[1] as u32) << 8) | ((a[2] as u32) << 16) | ((a[3] as u32) << 24)
}

/// Inserts a weighted index into a sorted array of 4, evicting the smallest
/// if the new item has higher weight. Matches C++ `insert_sort`.
fn insert_sort(sorted: &mut [WeightedIndex; 4], new_item: WeightedIndex) {
    if new_item.weight > sorted[0].weight {
        sorted[3] = sorted[2];
        sorted[2] = sorted[1];
        sorted[1] = sorted[0];
        sorted[0] = new_item;
    } else if new_item.weight > sorted[1].weight {
        sorted[3] = sorted[2];
        sorted[2] = sorted[1];
        sorted[1] = new_item;
    } else if new_item.weight > sorted[2].weight {
        sorted[3] = sorted[2];
        sorted[2] = new_item;
    } else if new_item.weight > sorted[3].weight {
        sorted[3] = new_item;
    }
}

/// Returns the position of `needle` in `haystack` (up to N entries), or 0
/// if not found. Matches C++ `index_of_or_zero`.
fn index_of_or_zero<const N: usize>(haystack: &[u8; 4], needle: u8) -> u8 {
    for (i, &entry) in haystack.iter().take(N).enumerate() {
        if entry == needle {
            return i as u8;
        }
    }
    0
}

/// Assigns component indices: for each voxel, finds which of the 4 selected
/// materials it uses.
fn assign_component_indices<const NVOXELS: usize>(
    available: &[u8; 4],
    cell_voxel_materials: &[u8; NVOXELS],
    component_indices: &mut [u8; NVOXELS],
) {
    for i in 0..NVOXELS {
        let mi = cell_voxel_materials[i];
        component_indices[i] = index_of_or_zero::<4>(available, mi);
    }
}

/// Selects the 4 most-represented materials in a cell of 8 voxels.
/// `voxel_material_indices` is the full channel slice; `voxel_indices` are
/// the 8 corner indices into it. Returns `CellMaterials` with packed data.
pub fn get_regular_cell_materials(
    voxel_material_indices: &[u8],
    voxel_corner_values: &[u8; 8],
) -> CellMaterials {
    let mut cell = CellMaterials::default();

    if voxel_material_indices.len() <= 1 {
        // Uniform channel → all same material.
        cell.selected_indices = [voxel_corner_values[0], 0, 0, 0];
        cell.packed_indices = pack_bytes(cell.selected_indices);
        // All components point to material 0.
        return cell;
    }

    // Count material occurrences using a simple histogram.
    let mut counts = [0u32; 256];
    for &v in voxel_corner_values {
        counts[v as usize] += 1;
    }

    // Find top 4 by weight using insertion sort.
    let mut sorted = [WeightedIndex::default(); 4];
    for (i, &weight) in counts.iter().enumerate() {
        if weight > 0 {
            insert_sort(
                &mut sorted,
                WeightedIndex {
                    index: i as u8,
                    weight,
                },
            );
        }
    }

    cell.selected_indices = [
        sorted[0].index,
        sorted[1].index,
        sorted[2].index,
        sorted[3].index,
    ];
    cell.packed_indices = pack_bytes(cell.selected_indices);
    assign_component_indices(
        &cell.selected_indices,
        voxel_corner_values,
        &mut cell.component_indices,
    );

    cell
}

/// Selects the 4 most-represented materials in a transition cell of 9 corners.
pub fn get_transition_cell_materials(voxel_corner_values: &[u8; 9]) -> TransitionCellMaterials {
    let mut cell = TransitionCellMaterials::default();

    // Count occurrences.
    let mut counts = [0u32; 256];
    for &v in voxel_corner_values {
        counts[v as usize] += 1;
    }

    let mut sorted = [WeightedIndex::default(); 4];
    for (i, &weight) in counts.iter().enumerate() {
        if weight > 0 {
            insert_sort(
                &mut sorted,
                WeightedIndex {
                    index: i as u8,
                    weight,
                },
            );
        }
    }

    cell.selected_indices = [
        sorted[0].index,
        sorted[1].index,
        sorted[2].index,
        sorted[3].index,
    ];
    cell.packed_indices = pack_bytes(cell.selected_indices);
    assign_component_indices(
        &cell.selected_indices,
        voxel_corner_values,
        &mut cell.component_indices,
    );

    cell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_cell_single_material() {
        let corner_values = [5u8; 8];
        let cell = get_regular_cell_materials(&[5; 100], &corner_values);
        assert_eq!(cell.selected_indices[0], 5);
        assert_eq!(cell.packed_indices, pack_bytes([5, 0, 0, 0]));
    }

    #[test]
    fn two_materials_selects_both() {
        let corner_values = [1u8, 1, 1, 1, 2, 2, 2, 2];
        let channel = [1u8, 2];
        let cell = get_regular_cell_materials(&channel, &corner_values);
        assert!(cell.selected_indices.contains(&1));
        assert!(cell.selected_indices.contains(&2));
    }

    #[test]
    fn pack_bytes_round_trips() {
        let packed = pack_bytes([10, 20, 30, 40]);
        assert_eq!(packed & 0xFF, 10);
        assert_eq!((packed >> 8) & 0xFF, 20);
        assert_eq!((packed >> 16) & 0xFF, 30);
        assert_eq!((packed >> 24) & 0xFF, 40);
    }

    #[test]
    fn transition_cell_uniform() {
        let corners = [3u8; 9];
        let cell = get_transition_cell_materials(&corners);
        assert_eq!(cell.selected_indices[0], 3);
    }

    #[test]
    fn default_mode_is_none() {
        assert_eq!(TexturingMode::default(), TexturingMode::None);
    }
}
