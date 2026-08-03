//! Additional Godot classes for voxel-core types.
//!
//! `VoxelBufferGD` exposes a VoxelBuffer as a Godot RefCounted.
//! `VoxelInstancerGD` is a Node3D for scatter-based instance placement.

use std::sync::Arc;

use godot::prelude::*;
use voxel_core::instancing::scatter::{InstanceGenerator, RandomScatterGenerator};
use voxel_core::instancing::{InstanceLibrary, ScatterConfig};
use voxel_core::math::{Vector3f, Vector3i};
use voxel_core::storage::{SharedVoxelGenerator, VoxelBuffer, VoxelFormat};

// ---------------------------------------------------------------------------
// VoxelBufferGD — RefCounted wrapper around VoxelBuffer
// ---------------------------------------------------------------------------

/// A Godot `RefCounted` wrapping a [`VoxelBuffer`]. Exposes basic voxel
/// read/write to GDScript for testing and procedural generation.
#[derive(GodotClass)]
#[class(base = RefCounted, tool, rename = VoxelBuffer)]
pub struct VoxelBufferGD {
    base: Base<RefCounted>,
    buffer: VoxelBuffer,
}

#[godot_api]
impl IRefCounted for VoxelBufferGD {
    fn init(base: Base<RefCounted>) -> Self {
        let mut buffer = VoxelBuffer::with_size(Vector3i::splat(1));
        VoxelFormat::new().configure_buffer(&mut buffer);
        Self { base, buffer }
    }
}

/// Validate a GDScript-supplied voxel coordinate + channel against a buffer.
/// Out-of-range input must never reach voxel-core indexing: the workspace
/// builds with `panic = "abort"`, so a panic here would kill the whole Godot
/// process. Invalid calls are ignored (reads return the default, writes are
/// dropped); debug builds assert so misuse is caught during development.
fn is_valid_access(buffer: &VoxelBuffer, x: i32, y: i32, z: i32, channel: i32) -> bool {
    let size = buffer.size();
    let valid = x >= 0
        && y >= 0
        && z >= 0
        && x < size.x
        && y < size.y
        && z < size.z
        && channel >= 0
        && (channel as usize) < buffer.channel_count();
    debug_assert!(
        valid,
        "VoxelBuffer access out of range: pos=({}, {}, {}), channel={} (size={:?}, channels={})",
        x,
        y,
        z,
        channel,
        size,
        buffer.channel_count()
    );
    valid
}

/// Validate a GDScript-supplied channel index (see [`is_valid_access`]).
fn is_valid_channel(buffer: &VoxelBuffer, channel: i32) -> bool {
    let valid = channel >= 0 && (channel as usize) < buffer.channel_count();
    debug_assert!(
        valid,
        "VoxelBuffer channel out of range: {} (channels={})",
        channel,
        buffer.channel_count()
    );
    valid
}

#[godot_api]
impl VoxelBufferGD {
    #[func]
    fn create(&mut self, size_x: i32, size_y: i32, size_z: i32) {
        self.buffer = VoxelBuffer::with_size(Vector3i::new(size_x, size_y, size_z));
        VoxelFormat::new().configure_buffer(&mut self.buffer);
    }

    #[func]
    fn set_voxel(&mut self, x: i32, y: i32, z: i32, channel: i32, value: i64) {
        if !is_valid_access(&self.buffer, x, y, z, channel) {
            return;
        }
        self.buffer
            .set_voxel(value as u64, x, y, z, channel as usize);
    }

    #[func]
    fn get_voxel(&self, x: i32, y: i32, z: i32, channel: i32) -> i64 {
        if !is_valid_access(&self.buffer, x, y, z, channel) {
            return 0;
        }
        self.buffer.get_voxel(x, y, z, channel as usize) as i64
    }

    #[func]
    fn get_size_x(&self) -> i32 {
        self.buffer.size().x
    }

    #[func]
    fn get_size_y(&self) -> i32 {
        self.buffer.size().y
    }

    #[func]
    fn get_size_z(&self) -> i32 {
        self.buffer.size().z
    }

    #[func]
    fn fill_channel(&mut self, channel: i32, value: i64) {
        if !is_valid_channel(&self.buffer, channel) {
            return;
        }
        self.buffer.fill(value as u64, channel as usize);
    }

    #[func]
    fn clear_channel(&mut self, channel: i32, value: i64) {
        if !is_valid_channel(&self.buffer, channel) {
            return;
        }
        self.buffer.clear_channel(channel as usize, value as u64);
    }
}

impl VoxelBufferGD {
    /// Borrow the underlying engine-agnostic [`VoxelBuffer`]. Used by sibling
    /// binding classes (mesher resources, modifiers) that need direct access
    /// to run voxel-core logic without round-tripping through Godot calls.
    pub fn core_buffer(&self) -> &VoxelBuffer {
        &self.buffer
    }

    /// Mutably borrow the underlying [`VoxelBuffer`].
    pub fn core_buffer_mut(&mut self) -> &mut VoxelBuffer {
        &mut self.buffer
    }
}

// ---------------------------------------------------------------------------
// VoxelInstancerGD — Node3D for scatter-based instance placement
// ---------------------------------------------------------------------------

/// A Godot `Node3D` that scatters instances (trees, rocks, grass) on
/// a parent [`VoxelTerrain`](crate::terrain::VoxelTerrain) using
/// [`voxel_core::instancing`].
#[derive(GodotClass)]
#[class(base = Node3D, tool, rename = VoxelInstancer)]
pub struct VoxelInstancerGD {
    base: Base<Node3D>,
    /// Instance library (items to scatter).
    library: InstanceLibrary,
    /// Scatter config.
    config: ScatterConfig,
    /// Density multiplier.
    #[var]
    density_multiplier: f32,
}

#[godot_api]
impl INode3D for VoxelInstancerGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            library: InstanceLibrary::new(),
            config: ScatterConfig::default(),
            density_multiplier: 1.0,
        }
    }

    fn ready(&mut self) {
        godot_print!("VoxelInstancerGD ready");
    }
}

#[godot_api]
impl VoxelInstancerGD {
    /// Add a scatter item (returns item index).
    #[func]
    fn add_item(&mut self, name: GString, density: f64, min_scale: f64, max_scale: f64) -> i32 {
        let item = voxel_core::instancing::InstanceLibraryItem {
            name: name.to_string(),
            density: density as f32,
            min_scale: min_scale as f32,
            max_scale: max_scale as f32,
            ..Default::default()
        };
        self.library.add_item(item) as i32
    }

    /// Get the number of items in the library.
    #[func]
    fn get_item_count(&self) -> i32 {
        self.library.len() as i32
    }

    /// Set the random seed for scatter.
    #[func]
    fn set_seed(&mut self, seed: i64) {
        self.config.seed = seed as u32;
    }

    /// Generate instances from a VoxelBufferGD's surface.
    /// Extracts surface points where solid meets air, runs the scatter
    /// generator for each library item, returns total instance count.
    #[func]
    fn scatter_from_buffer(&mut self, buffer: Gd<RefCounted>) -> i32 {
        if self.library.is_empty() {
            return 0;
        }
        // Try to cast to VoxelBufferGD for direct field access.
        if let Ok(buf_gd) = buffer.clone().try_cast::<VoxelBufferGD>() {
            let bound = buf_gd.bind();
            let sx = bound.get_size_x();
            let sy = bound.get_size_y();
            let sz = bound.get_size_z();

            let mut positions = Vec::new();
            let mut normals = Vec::new();
            for z in 1..sz {
                for y in 1..sy {
                    for x in 1..sx {
                        let vt = bound.get_voxel(x, y, z, 0);
                        let vt_below = bound.get_voxel(x, y - 1, z, 0);
                        if vt != 0 && vt_below == 0 {
                            positions.push(Vector3f::new(x as f32, y as f32, z as f32));
                            normals.push(Vector3f::new(0.0, 1.0, 0.0));
                        }
                    }
                }
            }
            drop(bound);
            drop(buf_gd);

            if positions.is_empty() {
                return 0;
            }

            let mut total = 0;
            for (idx, item) in self.library.items.iter().enumerate() {
                let gen = RandomScatterGenerator {
                    density: item.density * self.density_multiplier,
                    min_scale: item.min_scale,
                    max_scale: item.max_scale,
                    snap_to_normal: item.snap_to_normal,
                };
                let result = gen.generate(&positions, &normals, idx as u32, &self.config);
                total += result.len();
            }
            return total as i32;
        }
        0
    }

    /// Generate instances from dummy surface positions (test/debug).
    /// Returns the total instance count.
    #[func]
    fn scatter_test(&self, count: i32) -> i32 {
        // A negative count would wrap to a huge usize and overflow the
        // allocation below, aborting the Godot process (panic = "abort").
        if count <= 0 || self.library.is_empty() {
            return 0;
        }
        let positions: Vec<Vector3f> = (0..count)
            .map(|i| Vector3f::new(i as f32, 0.0, 0.0))
            .collect();
        let normals = vec![Vector3f::new(0.0, 1.0, 0.0); count as usize];
        let gen = RandomScatterGenerator {
            density: self.library.items[0].density * self.density_multiplier,
            min_scale: self.library.items[0].min_scale,
            max_scale: self.library.items[0].max_scale,
            snap_to_normal: true,
        };
        let result = gen.generate(&positions, &normals, 0, &self.config);
        result.len() as i32
    }
}

// ---------------------------------------------------------------------------
// VoxelToolTerrainGD — RefCounted terrain editing tool
// ---------------------------------------------------------------------------

/// A Godot `RefCounted` that wraps a reference to a [`VoxelTerrain`](crate::terrain::VoxelTerrain)
/// for GDScript-callable terrain editing operations.
#[derive(GodotClass)]
#[class(base = RefCounted, tool, rename = VoxelToolTerrain)]
pub struct VoxelToolTerrainGD {
    base: Base<RefCounted>,
    /// Weak reference to the terrain node path.
    terrain_path: GString,
}

#[godot_api]
impl IRefCounted for VoxelToolTerrainGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            terrain_path: "..".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelToolTerrainGD {
    #[func]
    fn set_terrain_path(&mut self, path: GString) {
        self.terrain_path = path;
    }

    #[func]
    fn get_terrain_path(&self) -> GString {
        self.terrain_path.clone()
    }
}

// ---------------------------------------------------------------------------
// VoxelRaycastResultGD — RefCounted result container
// ---------------------------------------------------------------------------

/// Result of a voxel raycast. Contains hit position, previous position,
/// and distance along the ray.
#[derive(GodotClass)]
#[class(base = RefCounted, tool, rename = VoxelRaycastResult)]
pub struct VoxelRaycastResultGD {
    base: Base<RefCounted>,
    #[var]
    hit_x: i32,
    #[var]
    hit_y: i32,
    #[var]
    hit_z: i32,
    #[var]
    prev_x: i32,
    #[var]
    prev_y: i32,
    #[var]
    prev_z: i32,
    #[var]
    distance: f32,
    #[var]
    normal_x: i32,
    #[var]
    normal_y: i32,
    #[var]
    normal_z: i32,
}

#[godot_api]
impl IRefCounted for VoxelRaycastResultGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            hit_x: 0,
            hit_y: 0,
            hit_z: 0,
            prev_x: 0,
            prev_y: 0,
            prev_z: 0,
            distance: 0.0,
            normal_x: 0,
            normal_y: 0,
            normal_z: 0,
        }
    }
}

#[godot_api]
impl VoxelRaycastResultGD {
    /// Whether this result represents a valid hit (distance > 0 and a
    /// non-zero normal). A default-constructed result reports no hit.
    #[func]
    fn did_hit(&self) -> bool {
        self.distance > 0.0 && (self.normal_x != 0 || self.normal_y != 0 || self.normal_z != 0)
    }

    /// The hit position as a packed array [x, y, z].
    #[func]
    fn get_hit_position(&self) -> PackedInt32Array {
        PackedInt32Array::from(&[self.hit_x, self.hit_y, self.hit_z][..])
    }
}

// ---------------------------------------------------------------------------
// VoxelNodeGD — base Node3D for voxel volumes (VoxelNode equivalent)
// ---------------------------------------------------------------------------

/// Base Node3D for voxel volume nodes. Holds shared properties.
/// In C++ this is the base class for VoxelTerrain/VoxelLodTerrain.
/// In Rust, VoxelTerrain inherits Node3D directly, but this class
/// exists for API parity and future VoxelLodTerrain.
#[derive(GodotClass)]
#[class(base = Node3D, tool, rename = VoxelNode)]
pub struct VoxelNodeGD {
    base: Base<Node3D>,
    /// Whether the terrain streams blocks around viewers.
    #[var]
    auto_load: bool,
    /// Maximum view distance in voxels.
    #[var]
    max_view_distance: i64,
}

#[godot_api]
impl INode3D for VoxelNodeGD {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            auto_load: true,
            max_view_distance: 192,
        }
    }
}

#[godot_api]
impl VoxelNodeGD {
    /// Whether this node is currently streaming blocks.
    #[func]
    fn is_streaming(&self) -> bool {
        self.auto_load
    }

    /// The effective view distance in blocks (max_view_distance / 16, min 1).
    #[func]
    fn get_view_distance_blocks(&self) -> i64 {
        (self.max_view_distance / 16).max(1)
    }
}

// ---------------------------------------------------------------------------
// VoxelGeneratorGraphGD — Resource wrapper for GraphGenerator
// ---------------------------------------------------------------------------

/// A Godot `Resource` wrapping a graph-based terrain generator.
///
/// Wraps [`voxel_core::generators::graph::GraphGenerator`] — the functional
/// API builds graphs, compiles them via `CompiledGraph`, and samples the SDF
/// output at a world point, exercising the full graph generation pipeline
/// through the binding.
#[derive(GodotClass)]
#[class(base = Resource, tool, rename = VoxelGeneratorGraph)]
pub struct VoxelGeneratorGraphGD {
    base: Base<Resource>,
    /// Graph nodes serialized as a JSON string (save/load interchange; not
    /// parsed back — build graphs programmatically via `add_node`).
    graph_json: GString,
    /// The node graph under construction.
    graph: voxel_core::generators::graph::Graph,
    /// The lazily-built engine-agnostic graph generator.
    generator: Option<voxel_core::generators::graph::GraphGenerator>,
}

#[godot_api]
impl IResource for VoxelGeneratorGraphGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            graph_json: "{}".to_godot(),
            graph: voxel_core::generators::graph::Graph::new(),
            generator: None,
        }
    }
}

#[godot_api]
impl VoxelGeneratorGraphGD {
    #[func]
    fn get_graph_json(&self) -> GString {
        self.graph_json.clone()
    }

    /// Replace the graph JSON. Resets the cached generator (it will rebuild on
    /// the next `sample_*` call).
    #[func]
    fn set_graph_json(&mut self, json: GString) {
        self.graph_json = json;
        self.generator = None;
    }

    /// Build a sphere-SDF graph (center `(cx,cy,cz)`, radius `r`), compile it,
    /// and return the sampled signed distance at world point `(px,py,pz)`.
    /// Negative = inside the sphere. Returns `NaN` if the graph fails to
    /// compile (malformed topology).
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn sample_sphere_sdf(
        &mut self,
        cx: f32,
        cy: f32,
        cz: f32,
        r: f32,
        px: f32,
        py: f32,
        pz: f32,
    ) -> f32 {
        use voxel_core::generators::graph::{
            CompiledGraph, CompiledScratch, Graph, GraphInputs, GraphOutput, GraphPort, NodeKind,
        };
        // SdfSphere evaluates sdf_sphere(pos, ZERO, radius), so feed the
        // sample point relative to the sphere center as the position inputs.
        let mut graph = Graph::new();
        let nx = graph.push(NodeKind::Constant(px - cx));
        let ny = graph.push(NodeKind::Constant(py - cy));
        let nz = graph.push(NodeKind::Constant(pz - cz));
        let nr = graph.push(NodeKind::Constant(r));
        let sphere = graph.push(NodeKind::SdfSphere {
            x: Some(GraphPort {
                node: nx,
                output: 0,
            }),
            y: Some(GraphPort {
                node: ny,
                output: 0,
            }),
            z: Some(GraphPort {
                node: nz,
                output: 0,
            }),
            radius: Some(GraphPort {
                node: nr,
                output: 0,
            }),
        });
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort {
                node: sphere,
                output: 0,
            }),
        });
        // Cache the generator so the compiled graph is reused across calls.
        self.generator = Some(voxel_core::generators::graph::GraphGenerator::new(graph));
        let Ok(compiled) = CompiledGraph::compile(self.generator.as_ref().unwrap().graph()) else {
            return f32::NAN;
        };
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

    /// Returns the number of nodes in the currently-cached generator's graph,
    /// or 0 if no graph has been built yet.
    #[func]
    fn get_node_count(&self) -> i32 {
        self.generator
            .as_ref()
            .map(|g| g.graph().nodes().len() as i32)
            .unwrap_or(0)
    }

    /// Remove all nodes from the graph under construction.
    #[func]
    fn clear_graph(&mut self) {
        self.graph = voxel_core::generators::graph::Graph::new();
        self.generator = None;
    }

    /// Append a node to the graph under construction and return its id
    /// (usable as a port input of later nodes). Returns `-1` for an unknown
    /// kind.
    ///
    /// Kinds: `InputX`, `InputY`, `InputZ`, `Constant` (uses `value`),
    /// `Add`/`Subtract`/`Multiply`/`Divide`/`Min`/`Max` (ports `a`,`b`),
    /// `Sin`/`Cos`/`Abs`/`Sqrt`/`Floor`/`Fract` (port `a`),
    /// `SdfPlane` (`a`=y, `b`=height),
    /// `SdfSphere` (`a`=x, `b`=y, `c`=z, `d`=radius),
    /// `SdfBox` (`a`=x, `b`=y, `c`=z; cube half-extent = `value`),
    /// `SdfUnion`/`SdfSubtract` (ports `a`,`b`),
    /// `SdfSmoothUnion`/`SdfSmoothSubtract` (ports `a`,`b`; `value` =
    /// smoothness), `Noise2D` (`a`=x, `b`=y), `Noise3D` (`a`=x, `b`=y,
    /// `c`=z), `OutputSdf` (port `a`).
    ///
    /// Unconnected ports: pass `-1`.
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn add_node(&mut self, kind: GString, a: i64, b: i64, c: i64, d: i64, value: f32) -> i64 {
        use voxel_core::generators::graph::{GraphPort, NodeKind};
        let port = |id: i64| -> Option<GraphPort> { (id >= 0).then(|| GraphPort::new(id as u32)) };
        let (pa, pb, pc, pd) = (port(a), port(b), port(c), port(d));
        let k = match kind.to_string().as_str() {
            "InputX" => NodeKind::InputX,
            "InputY" => NodeKind::InputY,
            "InputZ" => NodeKind::InputZ,
            "Constant" => NodeKind::Constant(value),
            "Add" => NodeKind::Add { a: pa, b: pb },
            "Subtract" => NodeKind::Subtract { a: pa, b: pb },
            "Multiply" => NodeKind::Multiply { a: pa, b: pb },
            "Divide" => NodeKind::Divide { a: pa, b: pb },
            "Min" => NodeKind::Min { a: pa, b: pb },
            "Max" => NodeKind::Max { a: pa, b: pb },
            "Sin" => NodeKind::Sin { a: pa },
            "Cos" => NodeKind::Cos { a: pa },
            "Abs" => NodeKind::Abs { a: pa },
            "Sqrt" => NodeKind::Sqrt { a: pa },
            "Floor" => NodeKind::Floor { a: pa },
            "Fract" => NodeKind::Fract { a: pa },
            "SdfPlane" => NodeKind::SdfPlane { y: pa, height: pb },
            "SdfSphere" => NodeKind::SdfSphere {
                x: pa,
                y: pb,
                z: pc,
                radius: pd,
            },
            "SdfBox" => NodeKind::SdfBox {
                x: pa,
                y: pb,
                z: pc,
                size_x: value,
                size_y: value,
                size_z: value,
            },
            "SdfUnion" => NodeKind::SdfUnion { a: pa, b: pb },
            "SdfSubtract" => NodeKind::SdfSubtract { a: pa, b: pb },
            "SdfSmoothUnion" => NodeKind::SdfSmoothUnion {
                a: pa,
                b: pb,
                smoothness: value,
            },
            "SdfSmoothSubtract" => NodeKind::SdfSmoothSubtract {
                a: pa,
                b: pb,
                smoothness: value,
            },
            "Noise2D" => NodeKind::Noise2D {
                x: pa,
                y: pb,
                noise: voxel_core::generators::simple::NoiseConfig::default(),
            },
            "Noise3D" => NodeKind::Noise3D {
                x: pa,
                y: pb,
                z: pc,
                noise: voxel_core::generators::simple::NoiseConfig::default(),
            },
            "OutputSdf" => NodeKind::OutputSdf { a: pa },
            _ => return -1,
        };
        let id = self.graph.push(k);
        self.generator = None;
        id as i64
    }

    /// Number of nodes in the graph under construction.
    #[func]
    pub fn get_graph_node_count(&self) -> i32 {
        self.graph.nodes().len() as i32
    }

    /// Check that the graph under construction compiles (no cycles, no
    /// dangling ports). Returns `false` for an empty graph.
    #[func]
    fn compile_graph(&self) -> bool {
        !self.graph.nodes().is_empty()
            && voxel_core::generators::graph::CompiledGraph::compile(&self.graph).is_ok()
    }

    /// Construct the engine-agnostic graph generator from the graph under
    /// construction, so it can drive a `VoxelTerrain` through the `generator`
    /// property. An empty/invalid graph generates nothing.
    pub fn create_core_generator(&self) -> SharedVoxelGenerator {
        Arc::new(voxel_core::generators::graph::GraphGenerator::new(
            self.graph.clone(),
        ))
    }
}
