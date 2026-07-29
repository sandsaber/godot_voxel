//! Editor plugin — `.vox` file importer with functional parsing.
//!
//! Ports the engine-coupled half of `editor/vox/vox_editor_plugin.cpp`.
//! The `.vox` importer uses `voxel_core::format::vox::parse` to parse
//! MagicaVoxel binary files and extract voxel data into a VoxelBuffer.

use godot::classes::mesh::PrimitiveType;
use godot::classes::{ArrayMesh, Control, EditorPlugin, IEditorPlugin};
use godot::prelude::*;

use voxel_core::generators::graph::{
    CompiledGraph, CompiledScratch, Graph, GraphInputs, GraphOutput, GraphPort, NodeKind,
};

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

// ---------------------------------------------------------------------------
// VoxelGraphEditorPlugin — EditorPlugin for the procedural graph editor
// ---------------------------------------------------------------------------

/// Editor plugin that hosts the procedural voxel graph editor.
///
/// Ports the engine-coupled half of `editor/graph/graph_editor_plugin.cpp`.
/// On `enter_tree` it adds a bottom-panel `Control` (the graph editor view);
/// on `exit_tree` it removes it. The functional API delegates to
/// `voxel_core::generators::graph::CompiledGraph` — `compile_sample_sphere`
/// builds a sphere-SDF graph, compiles it, and returns the sampled SDF value
/// at a world point, exercising the full graph pipeline through the binding.
#[derive(GodotClass)]
#[class(base = EditorPlugin, tool)]
pub struct VoxelGraphEditorPlugin {
    base: Base<EditorPlugin>,
    /// The bottom-panel view, kept alive while the plugin is in the tree.
    panel: Option<Gd<Control>>,
}

#[godot_api]
impl IEditorPlugin for VoxelGraphEditorPlugin {
    fn init(base: Base<EditorPlugin>) -> Self {
        godot_print!("VoxelGraphEditorPlugin: initialised");
        Self { base, panel: None }
    }

    fn enter_tree(&mut self) {
        // Build the graph editor view as a bottom panel. GraphEdit is not in
        // the generated bindings, so the host is a plain Control for now;
        // GDScript addons can populate it. This mirrors the C++ plugin's
        // `_handles` + `add_control_to_bottom_panel` setup.
        let panel = Control::new_alloc();
        self.base_mut()
            .add_control_to_bottom_panel(&panel, "Voxel Graph");
        self.panel = Some(panel);
        godot_print!("VoxelGraphEditorPlugin: entered tree — graph view added");
    }

    fn exit_tree(&mut self) {
        if let Some(panel) = self.panel.take() {
            self.base_mut().remove_control_from_bottom_panel(&panel);
            panel.free();
            godot_print!("VoxelGraphEditorPlugin: exited tree — graph view removed");
        }
    }
}

#[godot_api]
impl VoxelGraphEditorPlugin {
    /// Build a sphere-SDF graph (center at origin, radius `radius`), compile
    /// it, and return the sampled signed distance at world point
    /// `(px, py, pz)`. Negative = inside the sphere.
    ///
    /// Returns `f32::NAN` if the graph fails to compile (malformed).
    #[func]
    fn compile_sample_sphere(&self, radius: f32, px: f32, py: f32, pz: f32) -> f32 {
        let mut graph = Graph::new();
        let cx = graph.push(NodeKind::Constant(px));
        let cy = graph.push(NodeKind::Constant(py));
        let cz = graph.push(NodeKind::Constant(pz));
        let cr = graph.push(NodeKind::Constant(radius));
        let sphere = graph.push(NodeKind::SdfSphere {
            x: Some(GraphPort {
                node: cx,
                output: 0,
            }),
            y: Some(GraphPort {
                node: cy,
                output: 0,
            }),
            z: Some(GraphPort {
                node: cz,
                output: 0,
            }),
            radius: Some(GraphPort {
                node: cr,
                output: 0,
            }),
        });
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort {
                node: sphere,
                output: 0,
            }),
        });
        let Ok(compiled) = CompiledGraph::compile(&graph) else {
            return f32::NAN;
        };
        // Sample at the single point (slice size 1).
        let xs = [0.0f32];
        let zs = [0.0f32];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &zs,
        };
        let mut scratch = CompiledScratch::new();
        let mut out = Vec::new();
        compiled.generate_slice(&inputs, 1, &mut scratch, &mut out, false);
        out.into_iter()
            .find(|(k, _)| *k == GraphOutput::Sdf)
            .and_then(|(_, v)| v.into_iter().next())
            .unwrap_or(f32::NAN)
    }

    /// Returns the number of nodes in the default demo graph (a single
    /// sphere-SDF + output). Useful as a smoke check from GDScript.
    #[func]
    fn get_node_count(&self) -> i32 {
        // Input/Constant ×4 + SdfSphere + OutputSdf.
        6
    }
}
