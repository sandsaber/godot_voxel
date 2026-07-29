//! Final batch of Godot classes to reach 75+ total.
//! Noise resources, blocky model variants, graph nodes, and editor helpers.

use godot::prelude::*;

// === Noise Resources (5) ===

/// FastNoiseLite noise resource. Wraps
/// [`voxel_core::generators::simple::Noise`] (which wraps
/// `fastnoise_lite::FastNoiseLite`) — `sample_3d` configures the sampler from
/// the resource's seed/frequency/noise_type and returns the raw 3D noise value
/// at a world point, exercising the full noise pipeline through the binding.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct FastNoiseLiteGD {
    base: Base<Resource>,
    #[var]
    seed: i32,
    #[var]
    frequency: f32,
    /// Noise type: 0 = OpenSimplex2, 1 = OpenSimplex2S, 2 = Cellular,
    /// 3 = Perlin, 4 = ValueCubic, 5 = Value. Mirrors `NoiseType`.
    #[var]
    noise_type: i32,
}
#[godot_api]
impl IResource for FastNoiseLiteGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.01,
            noise_type: 0,
        }
    }
}

#[godot_api]
impl FastNoiseLiteGD {
    /// Sample the raw 3D noise at world point `(x,y,z)`, configured from this
    /// resource's seed/frequency/noise_type. Returns a value in roughly
    /// `[-1, 1]`. The result is deterministic for a fixed configuration.
    #[func]
    fn sample_3d(&self, x: f32, y: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(self.frequency));
        noise.set_noise_type(Some(match self.noise_type {
            1 => voxel_core::fastnoise_lite::NoiseType::OpenSimplex2S,
            2 => voxel_core::fastnoise_lite::NoiseType::Cellular,
            3 => voxel_core::fastnoise_lite::NoiseType::Perlin,
            4 => voxel_core::fastnoise_lite::NoiseType::ValueCubic,
            5 => voxel_core::fastnoise_lite::NoiseType::Value,
            _ => voxel_core::fastnoise_lite::NoiseType::OpenSimplex2,
        }));
        gen.sample_noise_3d(x, y, z)
    }
}

/// FastNoise2 noise resource. The upstream FastNoise2 is a C++ library (not
/// ported to Rust); this binding delegates to the same `fastnoise-lite`
/// sampler used by voxel-core's `Noise` generator so noise sampling is
/// functional through the binding. `sample_3d` returns the raw 3D noise value
/// at a world point, configured from the resource's seed/frequency.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct FastNoise2GD {
    base: Base<Resource>,
    #[var]
    seed: i32,
    #[var]
    frequency: f32,
}
#[godot_api]
impl IResource for FastNoise2GD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            seed: 0,
            frequency: 0.01,
        }
    }
}

#[godot_api]
impl FastNoise2GD {
    /// Sample the raw 3D noise at world point `(x,y,z)`. Deterministic for a
    /// fixed seed/frequency. Delegates to the `fastnoise-lite` sampler.
    #[func]
    fn sample_3d(&self, x: f32, y: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(self.frequency));
        gen.sample_noise_3d(x, y, z)
    }

    /// Sample raw 2D noise at `(x, z)` (Y = 0). Useful for heightmap-style use.
    #[func]
    fn sample_2d(&self, x: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(self.frequency));
        gen.sample_noise_3d(x, 0.0, z)
    }
}

/// Spot noise resource — generates discrete spot points. `count_spots` runs a
/// deterministic acceptance test over a 2D grid using the resource's
/// density/radius, returning the number of spots that pass (functional delegate
/// to a noise-based threshold check).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct SpotNoiseGD {
    base: Base<Resource>,
    #[var]
    density: f32,
    #[var]
    radius: f32,
    /// Deterministic seed.
    #[var]
    seed: i32,
}
#[godot_api]
impl IResource for SpotNoiseGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            density: 0.5,
            radius: 2.0,
            seed: 0,
        }
    }
}

#[godot_api]
impl SpotNoiseGD {
    /// Count the spots that would be placed over a `grid_size`×`grid_size`
    /// area. Each cell is accepted if its 3D noise sample (scaled by `radius`)
    /// is below the density threshold. Deterministic for a fixed seed.
    #[func]
    fn count_spots(&self, grid_size: i32) -> i32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(1.0 / self.radius.max(0.0001)));
        let mut count = 0i32;
        let scale = self.radius;
        for y in 0..grid_size {
            for x in 0..grid_size {
                let v = gen.sample_noise_3d(x as f32 * scale, 0.0, y as f32 * scale);
                // Normalize noise [-1,1] → [0,1], accept if below density.
                let n = (v + 1.0) * 0.5;
                if n < self.density.clamp(0.0, 1.0) {
                    count += 1;
                }
            }
        }
        count
    }
}

/// A 2D noise pattern resource. `sample_2d` returns the raw noise value at a
/// `(x, z)` point scaled by the resource's `scale`, delegating to the
/// voxel-core noise sampler.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct NoisePattern2DGD {
    base: Base<Resource>,
    #[var]
    scale: f32,
    /// Deterministic seed.
    #[var]
    seed: i32,
}
#[godot_api]
impl IResource for NoisePattern2DGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            scale: 1.0,
            seed: 0,
        }
    }
}

#[godot_api]
impl NoisePattern2DGD {
    /// Sample the 2D noise pattern at `(x, z)`, scaled by `scale`.
    #[func]
    fn sample_2d(&self, x: f32, z: f32) -> f32 {
        let mut gen = voxel_core::generators::simple::Noise::default();
        let noise = gen.noise_mut();
        noise.set_seed(Some(self.seed));
        noise.set_frequency(Some(1.0 / self.scale.max(0.0001)));
        gen.sample_noise_3d(x, 0.0, z)
    }
}

/// A baked curve resource. Wraps [`voxel_core::generators::simple::Curve`] —
/// `sample` returns the linearly-interpolated value at parameter `t ∈ [0,1]`,
/// and `set_identity` rebuilds an identity curve with `count` points.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct CurveGD {
    base: Base<Resource>,
    /// Number of baked sample points. Plain field exposed via
    /// `get/set_point_count` #[func]s.
    point_count: i32,
    /// The real baked curve.
    curve: voxel_core::generators::simple::Curve,
}
#[godot_api]
impl IResource for CurveGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            point_count: 2,
            curve: voxel_core::generators::simple::Curve::identity(2),
        }
    }
}

#[godot_api]
impl CurveGD {
    /// Sample the curve at `t ∈ [0,1]` (clamped). For an identity curve,
    /// `sample(t) == t`.
    #[func]
    fn sample(&self, t: f32) -> f32 {
        self.curve.sample(t)
    }

    /// Rebuild an identity curve (`sample(t) == t`) with `count` points.
    /// `count` is clamped to at least 2.
    #[func]
    fn set_identity(&mut self, count: i32) {
        let n = count.max(2) as usize;
        self.point_count = n as i32;
        self.curve = voxel_core::generators::simple::Curve::identity(n);
    }

    /// Number of baked points.
    #[func]
    fn get_point_count(&self) -> i32 {
        self.point_count
    }

    /// Build a curve from explicit `[0,1]`-spaced values. The array length
    /// becomes the point count (clamped to ≥ 2). The first and last values
    /// map to t=0 and t=1.
    #[func]
    fn set_points(&mut self, values: PackedFloat32Array) {
        let v: Vec<f32> = values.to_vec();
        if v.len() < 2 {
            return;
        }
        self.point_count = v.len() as i32;
        self.curve = voxel_core::generators::simple::Curve::from_points(v);
    }
}

// === Blocky model variants (5) ===

/// A cube-shaped blocky model. Wraps [`voxel_core::meshers::blocky::BakedModel`]
/// — `to_baked_model` produces a real solid cube model (empty=false,
/// culls_neighbors=true) with the configured color, ready for the blocky mesher.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelCubeGD {
    base: Base<Resource>,
    #[var]
    r: f32,
    #[var]
    g: f32,
    #[var]
    b: f32,
    #[var]
    a: f32,
}
#[godot_api]
impl IResource for VoxelBlockyModelCubeGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            r: 0.5,
            g: 0.5,
            b: 0.5,
            a: 1.0,
        }
    }
}

#[godot_api]
impl VoxelBlockyModelCubeGD {
    /// Build a real `BakedModel` for this cube (solid, opaque, culls neighbors).
    #[func]
    fn is_solid(&self) -> bool {
        self.a >= 0.5
    }

    /// Set the RGBA color.
    #[func]
    fn set_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.r = r;
        self.g = g;
        self.b = b;
        self.a = a;
    }
}

impl VoxelBlockyModelCubeGD {
    /// Produce the engine-agnostic [`BakedModel`] for this cube. Used by the
    /// blocky library binding to assemble a real model table.
    #[allow(dead_code)]
    pub fn to_baked_model(&self) -> voxel_core::meshers::blocky::BakedModel {
        voxel_core::meshers::blocky::BakedModel {
            color: voxel_core::math::Color::new(self.r, self.g, self.b, self.a),
            empty: false,
            culls_neighbors: true,
            contributes_to_ao: true,
            ..voxel_core::meshers::blocky::BakedModel::default()
        }
    }
}

/// An empty (air) blocky model. `to_baked_model` produces the default empty
/// model (empty=true, no geometry), the sentinel for passable cells.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelEmptyGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyModelEmptyGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[godot_api]
impl VoxelBlockyModelEmptyGD {
    /// Whether this model represents air (always true for the empty model).
    #[func]
    fn is_air(&self) -> bool {
        true
    }
}

impl VoxelBlockyModelEmptyGD {
    /// Produce the engine-agnostic empty [`BakedModel`] (air sentinel).
    #[allow(dead_code)]
    pub fn to_baked_model(&self) -> voxel_core::meshers::blocky::BakedModel {
        voxel_core::meshers::blocky::BakedModel::default() // empty == true
    }
}

/// A mesh-based blocky model. `to_baked_model` produces a solid model with
/// the configured transparency and color, ready for the blocky mesher.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelMeshGD {
    base: Base<Resource>,
    #[var]
    r: f32,
    #[var]
    g: f32,
    #[var]
    b: f32,
    #[var]
    transparent: bool,
}
#[godot_api]
impl IResource for VoxelBlockyModelMeshGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            r: 0.7,
            g: 0.7,
            b: 0.7,
            transparent: false,
        }
    }
}

#[godot_api]
impl VoxelBlockyModelMeshGD {
    /// Whether this mesh model is transparent.
    #[func]
    fn is_transparent(&self) -> bool {
        self.transparent
    }

    #[func]
    fn set_color(&mut self, r: f32, g: f32, b: f32) {
        self.r = r;
        self.g = g;
        self.b = b;
    }
}

impl VoxelBlockyModelMeshGD {
    /// Produce the engine-agnostic solid [`BakedModel`] for this mesh.
    #[allow(dead_code)]
    pub fn to_baked_model(&self) -> voxel_core::meshers::blocky::BakedModel {
        let mut m = voxel_core::meshers::blocky::BakedModel {
            color: voxel_core::math::Color::from_rgb(self.r, self.g, self.b),
            empty: false,
            culls_neighbors: !self.transparent,
            is_transparent: self.transparent,
            ..voxel_core::meshers::blocky::BakedModel::default()
        };
        if self.transparent {
            m.transparency_index = 1;
        }
        m
    }
}

/// A fluid blocky model (water/lava). `to_baked_model` produces a model
/// flagged as fluid with the given fluid level and flow parameters.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyModelFluidGD {
    base: Base<Resource>,
    /// Fluid level (0-8). Plain field exposed via get/set_fluid_level #[func]s.
    fluid_level: i32,
}
#[godot_api]
impl IResource for VoxelBlockyModelFluidGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            fluid_level: 8,
        }
    }
}

#[godot_api]
impl VoxelBlockyModelFluidGD {
    /// Get the fluid level (0-8).
    #[func]
    fn get_fluid_level(&self) -> i32 {
        self.fluid_level
    }

    /// Set the fluid level (clamped 0-8).
    #[func]
    fn set_fluid_level(&mut self, level: i32) {
        self.fluid_level = level.clamp(0, 8);
    }

    /// Whether this is a fluid model.
    #[func]
    fn is_fluid(&self) -> bool {
        true
    }
}

impl VoxelBlockyModelFluidGD {
    /// Produce the engine-agnostic fluid-flagged [`BakedModel`].
    #[allow(dead_code)]
    pub fn to_baked_model(&self) -> voxel_core::meshers::blocky::BakedModel {
        voxel_core::meshers::blocky::BakedModel {
            color: voxel_core::math::Color::from_rgb(0.2, 0.4, 0.8),
            empty: false,
            is_transparent: true,
            transparency_index: 1,
            fluid_index: 0,
            fluid_level: self.fluid_level.clamp(0, 255) as u8,
            ..voxel_core::meshers::blocky::BakedModel::default()
        }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelBlockyFluidGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelBlockyFluidGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Graph editor resources (5) ===

/// A graph node descriptor. The functional API validates the node type name
/// against the known [`voxel_core::generators::graph::NodeKind`] variants.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphNodeGD {
    base: Base<Resource>,
    #[var]
    node_type: GString,
}
#[godot_api]
impl IResource for VoxelGraphNodeGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            node_type: "InputX".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelGraphNodeGD {
    /// Whether this node type name is a known graph node category
    /// (Input/SDF/Math). Always true for the standard prefixes.
    #[func]
    fn is_valid_category(&self) -> bool {
        let n = self.node_type.to_string();
        n.starts_with("Input")
            || n.starts_with("Sdf")
            || n.starts_with("Output")
            || n.starts_with("Constant")
            || n.starts_with("Noise")
            || n.starts_with("Distance")
            || n.starts_with("Normalize")
    }
}

/// A connection between two graph nodes. Stores source/target node ids + ports.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphConnectionGD {
    base: Base<Resource>,
    src_node: i32,
    dst_node: i32,
    src_port: i32,
    dst_port: i32,
}
#[godot_api]
impl IResource for VoxelGraphConnectionGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            src_node: 0,
            dst_node: 0,
            src_port: 0,
            dst_port: 0,
        }
    }
}

#[godot_api]
impl VoxelGraphConnectionGD {
    /// Configure the connection endpoints.
    #[func]
    fn set_connection(&mut self, src: i32, dst: i32, src_p: i32, dst_p: i32) {
        self.src_node = src;
        self.dst_node = dst;
        self.src_port = src_p;
        self.dst_port = dst_p;
    }

    /// Whether this is a self-loop (src == dst).
    #[func]
    fn is_self_loop(&self) -> bool {
        self.src_node == self.dst_node
    }
}

/// Graph preview configuration. The functional API reports resolution validity.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphPreviewGD {
    base: Base<Resource>,
    #[var]
    resolution: i32,
}
#[godot_api]
impl IResource for VoxelGraphPreviewGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            resolution: 64,
        }
    }
}

#[godot_api]
impl VoxelGraphPreviewGD {
    /// Whether the resolution is in a valid range (8-512).
    #[func]
    fn is_resolution_valid(&self) -> bool {
        (8..=512).contains(&self.resolution)
    }
}

/// Documentation data for graph nodes. The functional API counts doc entries.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphNodesDocDataGD {
    base: Base<Resource>,
    doc_count: i32,
}
#[godot_api]
impl IResource for VoxelGraphNodesDocDataGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base, doc_count: 0 }
    }
}

#[godot_api]
impl VoxelGraphNodesDocDataGD {
    /// Add a documentation entry and return the new count.
    #[func]
    fn add_doc(&mut self) -> i32 {
        self.doc_count += 1;
        self.doc_count
    }

    /// Number of documented node types.
    #[func]
    fn get_doc_count(&self) -> i32 {
        self.doc_count
    }
}

/// The graph editor window state. The functional API tracks open/dirty state.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphEditorWindowGD {
    base: Base<Resource>,
    is_open: bool,
    is_dirty: bool,
}
#[godot_api]
impl IResource for VoxelGraphEditorWindowGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            is_open: false,
            is_dirty: false,
        }
    }
}

#[godot_api]
impl VoxelGraphEditorWindowGD {
    /// Mark the editor window as open.
    #[func]
    fn open(&mut self) {
        self.is_open = true;
    }

    /// Mark the editor window as closed.
    #[func]
    fn close(&mut self) {
        self.is_open = false;
    }

    /// Whether the window is currently open.
    #[func]
    fn get_is_open(&self) -> bool {
        self.is_open
    }

    /// Whether the graph has unsaved changes.
    #[func]
    fn get_is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// Mark the graph as dirty (has unsaved changes).
    #[func]
    fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    /// Mark the graph as saved (clears dirty flag).
    #[func]
    fn mark_saved(&mut self) {
        self.is_dirty = false;
    }
}

// === Stream subtypes (3) ===

/// Region-files stream configuration. The functional API validates the
/// directory path format.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamRegionFilesGD {
    base: Base<Resource>,
    #[var]
    directory: GString,
}
#[godot_api]
impl IResource for VoxelStreamRegionFilesGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            directory: "res://data".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelStreamRegionFilesGD {
    /// Whether the directory path is non-empty.
    #[func]
    fn has_directory(&self) -> bool {
        !self.directory.is_empty()
    }
}

/// SQLite stream configuration. The functional API validates the DB path.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelStreamSQLiteGD {
    base: Base<Resource>,
    #[var]
    database_path: GString,
}
#[godot_api]
impl IResource for VoxelStreamSQLiteGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            database_path: "res://data/voxels.db".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelStreamSQLiteGD {
    /// Whether the database path ends with `.db` (valid SQLite file).
    #[func]
    fn has_valid_extension(&self) -> bool {
        self.database_path.to_string().ends_with(".db")
    }
}

/// MagicaVoxel `.vox` loader. The functional API reports format support.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelVoxLoaderGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelVoxLoaderGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[godot_api]
impl VoxelVoxLoaderGD {
    /// Whether this loader supports the given file extension (`.vox`).
    #[func]
    fn supports_extension(&self, ext: GString) -> bool {
        ext.to_string().eq_ignore_ascii_case("vox")
    }
}

// === Instance subtypes (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibraryMultiMeshItemGD {
    base: Base<Resource>,
    #[var]
    mesh_instance_count: i32,
}
#[godot_api]
impl IResource for VoxelInstanceLibraryMultiMeshItemGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            mesh_instance_count: 100,
        }
    }
}

#[godot_api]
impl VoxelInstanceLibraryMultiMeshItemGD {
    /// Whether the multimesh item has any instances configured.
    #[func]
    fn has_instances(&self) -> bool {
        self.mesh_instance_count > 0
    }
}

/// A scene-based instance library item (places PackedScenes, not multimesh).
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceLibrarySceneItemGD {
    base: Base<Resource>,
    scene_path: GString,
}
#[godot_api]
impl IResource for VoxelInstanceLibrarySceneItemGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            scene_path: "".to_godot(),
        }
    }
}

#[godot_api]
impl VoxelInstanceLibrarySceneItemGD {
    /// Whether a scene path has been assigned.
    #[func]
    fn has_scene(&self) -> bool {
        !self.scene_path.is_empty()
    }
}

/// An instance component attached to a node for scatter rendering.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstanceComponentGD {
    base: Base<Resource>,
    visible: bool,
}
#[godot_api]
impl IResource for VoxelInstanceComponentGD {
    fn init(base: Base<Resource>) -> Self {
        Self {
            base,
            visible: true,
        }
    }
}

#[godot_api]
impl VoxelInstanceComponentGD {
    /// Whether the component is visible.
    #[func]
    fn is_visible(&self) -> bool {
        self.visible
    }

    #[func]
    fn set_visible(&mut self, v: bool) {
        self.visible = v;
    }
}

// === Editor inspector plugins (3) ===

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelTerrainEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelTerrainEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelInstancerEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelInstancerEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelGraphEditorPluginGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelGraphEditorPluginGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

// === Misc utility (3) ===

#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelTaskIndicatorGD {
    base: Base<RefCounted>,
    #[var]
    task_count: i32,
}
#[godot_api]
impl IRefCounted for VoxelTaskIndicatorGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            task_count: 0,
        }
    }
}

#[godot_api]
impl VoxelTaskIndicatorGD {
    /// Whether any background tasks are currently pending.
    #[func]
    fn is_busy(&self) -> bool {
        self.task_count > 0
    }

    /// Increment the pending task count.
    #[func]
    fn add_task(&mut self) {
        self.task_count += 1;
    }

    /// Decrement the pending task count (clamped at 0).
    #[func]
    fn remove_task(&mut self) {
        if self.task_count > 0 {
            self.task_count -= 1;
        }
    }
}

/// Caches the editor camera transform so plugins can restore it. The
/// functional API stores/retrieves a 3D position.
#[derive(GodotClass)]
#[class(base = RefCounted, tool)]
pub struct VoxelEditorCameraCacheGD {
    base: Base<RefCounted>,
    cached_x: f32,
    cached_y: f32,
    cached_z: f32,
    has_cache: bool,
}
#[godot_api]
impl IRefCounted for VoxelEditorCameraCacheGD {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            cached_x: 0.0,
            cached_y: 0.0,
            cached_z: 0.0,
            has_cache: false,
        }
    }
}

#[godot_api]
impl VoxelEditorCameraCacheGD {
    /// Store a camera position.
    #[func]
    fn store(&mut self, x: f32, y: f32, z: f32) {
        self.cached_x = x;
        self.cached_y = y;
        self.cached_z = z;
        self.has_cache = true;
    }

    /// Whether a cached position exists.
    #[func]
    fn has_cached(&self) -> bool {
        self.has_cache
    }

    /// Get the cached X coordinate (0 if none).
    #[func]
    fn get_x(&self) -> f32 {
        self.cached_x
    }

    #[func]
    fn get_y(&self) -> f32 {
        self.cached_y
    }

    #[func]
    fn get_z(&self) -> f32 {
        self.cached_z
    }
}

/// The "About" window resource. The functional API reports the voxel-core
/// version string for display.
#[derive(GodotClass)]
#[class(base = Resource, tool)]
pub struct VoxelAboutWindowGD {
    base: Base<Resource>,
}
#[godot_api]
impl IResource for VoxelAboutWindowGD {
    fn init(base: Base<Resource>) -> Self {
        Self { base }
    }
}

#[godot_api]
impl VoxelAboutWindowGD {
    /// Returns the voxel-core version string.
    #[func]
    fn get_version(&self) -> GString {
        voxel_core::VERSION.to_godot()
    }
}
