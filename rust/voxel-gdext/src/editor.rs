//! Editor plugin — `.vox` file importer with functional parsing.
//!
//! Ports the engine-coupled half of `editor/vox/vox_editor_plugin.cpp`.
//! The `.vox` importer uses `voxel_core::format::vox::parse` to parse
//! MagicaVoxel binary files and extract voxel data into a VoxelBuffer.

use godot::classes::mesh::PrimitiveType;
use godot::classes::{ArrayMesh, EditorPlugin, IEditorPlugin};
use godot::prelude::*;

/// Editor plugin for importing `.vox` (MagicaVoxel) files.
/// Parses the binary format via `voxel_core::format::vox::parse` and
/// converts the first model into an ArrayMesh.
#[derive(GodotClass)]
#[class(base = EditorPlugin, tool)]
pub struct VoxImporterPlugin {
    base: Base<EditorPlugin>,
}

#[godot_api]
impl IEditorPlugin for VoxImporterPlugin {
    fn init(base: Base<EditorPlugin>) -> Self {
        godot_print!("VoxImporterPlugin: initialised");
        Self { base }
    }

    fn enter_tree(&mut self) {
        godot_print!("VoxImporterPlugin: entered tree — .vox import available");
    }

    fn exit_tree(&mut self) {
        godot_print!("VoxImporterPlugin: exited tree");
    }
}

#[godot_api]
impl VoxImporterPlugin {
    /// Parse a `.vox` file from raw bytes and return the first model's
    /// voxel data as a flat `[positions_x3, colors_r, colors_g, colors_b]`
    /// PackedFloat32Array. Each voxel is 6 floats (x, y, z, r, g, b).
    ///
    /// This is callable from GDScript:
    /// ```gdscript
    /// var plugin = VoxImporterPlugin.new()
    /// var data = plugin.parse_vox_bytes(file.get_buffer(file.get_length()))
    /// ```
    #[func]
    fn parse_vox_bytes(&self, bytes: PackedByteArray) -> PackedFloat32Array {
        let raw = bytes.as_slice();
        match voxel_core::format::vox::parse(raw) {
            Ok(data) => {
                let mut result = Vec::new();
                if let Some(model) = data.models.first() {
                    let sx = model.size.x as usize;
                    let sy = model.size.y as usize;
                    let sz = model.size.z as usize;
                    for voxel_x in 0..sx {
                        for voxel_y in 0..sy {
                            for voxel_z in 0..sz {
                                let idx = voxel_y + sy * (voxel_x + sx * voxel_z);
                                let ci = model.color_indexes.get(idx).copied().unwrap_or(0);
                                if ci == 0 {
                                    continue;
                                }
                                let c = data.palette[ci as usize];
                                result.push(voxel_x as f32);
                                result.push(voxel_y as f32);
                                result.push(voxel_z as f32);
                                result.push(c.r as f32 / 255.0);
                                result.push(c.g as f32 / 255.0);
                                result.push(c.b as f32 / 255.0);
                            }
                        }
                    }
                    return PackedFloat32Array::from(result.as_slice());
                }
                PackedFloat32Array::from(result.as_slice())
            }
            Err(e) => {
                godot_print!("VoxImporterPlugin: parse error: {e:?}");
                PackedFloat32Array::new()
            }
        }
    }

    /// Parse a `.vox` file and build an ArrayMesh from the voxel data.
    /// Each voxel becomes a cube face quad. Returns the ArrayMesh.
    #[func]
    fn parse_vox_to_mesh(&self, bytes: PackedByteArray) -> Gd<ArrayMesh> {
        let raw = bytes.as_slice();
        let mut mesh = ArrayMesh::new_gd();
        match voxel_core::format::vox::parse(raw) {
            Ok(data) => {
                if let Some(model) = data.models.first() {
                    let mut positions: Vec<Vector3> = Vec::new();
                    let mut normals: Vec<Vector3> = Vec::new();
                    let mut indices: Vec<i32> = Vec::new();
                    let mut vi = 0i32;

                    let sx = model.size.x as usize;
                    let sy = model.size.y as usize;
                    let sz = model.size.z as usize;
                    for zi in 0..sz {
                        for xi in 0..sx {
                            for yi in 0..sy {
                                let idx = yi + sy * (xi + sx * zi);
                                let ci = model.color_indexes.get(idx).copied().unwrap_or(0);
                                if ci == 0 {
                                    continue;
                                }
                                let x = xi as f32;
                                let y = yi as f32;
                                let z = zi as f32;
                                let c = data.palette[ci as usize];
                                let _ = c; // Color8 available for material/vertex colors

                                // 6 faces × 4 verts = 24 verts per voxel.
                                let faces: [(Vector3, [[f32; 3]; 4]); 6] = [
                                    // +X face
                                    (
                                        Vector3::new(1.0, 0.0, 0.0),
                                        [
                                            [x + 1.0, y, z],
                                            [x + 1.0, y + 1.0, z],
                                            [x + 1.0, y + 1.0, z + 1.0],
                                            [x + 1.0, y, z + 1.0],
                                        ],
                                    ),
                                    // -X face
                                    (
                                        Vector3::new(-1.0, 0.0, 0.0),
                                        [
                                            [x, y, z + 1.0],
                                            [x, y + 1.0, z + 1.0],
                                            [x, y + 1.0, z],
                                            [x, y, z],
                                        ],
                                    ),
                                    // +Y face (top)
                                    (
                                        Vector3::new(0.0, 1.0, 0.0),
                                        [
                                            [x, y + 1.0, z],
                                            [x, y + 1.0, z + 1.0],
                                            [x + 1.0, y + 1.0, z + 1.0],
                                            [x + 1.0, y + 1.0, z],
                                        ],
                                    ),
                                    // -Y face (bottom)
                                    (
                                        Vector3::new(0.0, -1.0, 0.0),
                                        [
                                            [x, y, z + 1.0],
                                            [x, y, z],
                                            [x + 1.0, y, z],
                                            [x + 1.0, y, z + 1.0],
                                        ],
                                    ),
                                    // +Z face
                                    (
                                        Vector3::new(0.0, 0.0, 1.0),
                                        [
                                            [x, y, z + 1.0],
                                            [x + 1.0, y, z + 1.0],
                                            [x + 1.0, y + 1.0, z + 1.0],
                                            [x, y + 1.0, z + 1.0],
                                        ],
                                    ),
                                    // -Z face
                                    (
                                        Vector3::new(0.0, 0.0, -1.0),
                                        [
                                            [x + 1.0, y, z],
                                            [x, y, z],
                                            [x, y + 1.0, z],
                                            [x + 1.0, y + 1.0, z],
                                        ],
                                    ),
                                ];

                                for (normal, verts) in &faces {
                                    for v in verts.iter() {
                                        positions.push(Vector3::new(v[0], v[1], v[2]));
                                        normals.push(*normal);
                                    }
                                    indices.extend_from_slice(&[
                                        vi,
                                        vi + 1,
                                        vi + 2,
                                        vi,
                                        vi + 2,
                                        vi + 3,
                                    ]);
                                    vi += 4;
                                }
                            }
                        }
                    }

                    let mut arrays = Array::new();
                    arrays.push(&PackedVector3Array::from(positions.as_slice()));
                    arrays.push(&PackedVector3Array::from(normals.as_slice()));
                    for _ in 2..12 {
                        arrays.push(&Variant::nil());
                    }
                    arrays.push(&PackedInt32Array::from(indices.as_slice()));
                    mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);
                    let voxel_count = positions.len() / 24;
                    godot_print!(
                        "VoxImporterPlugin: parsed {voxel_count} voxels → {} vertices",
                        positions.len()
                    );
                }
            }
            Err(e) => {
                godot_print!("VoxImporterPlugin: parse error: {e:?}");
            }
        }
        mesh
    }
}
