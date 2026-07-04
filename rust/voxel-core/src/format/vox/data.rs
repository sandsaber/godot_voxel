//! Data types for the MagicaVoxel `.vox` scene: models, scene-graph nodes,
//! layers, materials and the 256-color palette.
//!
//! Ported from `streams/vox/vox_data.h`. The C++ version uses inheritance
//! (`Node` base + `TransformNode`/`GroupNode`/`ShapeNode` subclasses stored as
//! `UniquePtr<Node>` in an `unordered_map<int, ...>`). Rust expresses the same
//! as a tagged [`Node`] enum keyed by id in a [`HashMap`], which removes the
//! `reinterpret_cast` downcasts the C++ does in the validation pass.
//!
//! Coordinate conventions and the default palette are shared with [`parser`];
//! see that module for the format spec links.

use std::collections::HashMap;

use crate::math::{Basis3f, Color8, Vector3i};

/// Maximum dimension of a voxel model (MagicaVoxel hard cap).
pub const MAX_MODEL_SIZE: i32 = 256;

/// Number of entries in a `.vox` palette.
pub const PALETTE_SIZE: usize = 256;

/// A voxel model: an axis-aligned box of color-index bytes.
///
/// `color_indexes` is laid out in ZXY order (`y + sy*(x + sx*z)`, Y innermost),
/// matching the rest of the engine — see [`Vector3i::zxy_index`].
///
/// Ported from `magica::Model`. The C++ "lazy loading" TODO is dropped; a
/// `Vec<u8>` is already heap-allocated and lazily touched.
#[derive(Debug, Clone, Default)]
pub struct Model {
    pub size: Vector3i,
    pub color_indexes: Vec<u8>,
}

/// A rotation byte as stored in `nTRN._r`, decoded into a [`Basis3f`].
///
/// Ported from `magica::Rotation`.
#[derive(Debug, Clone, Default)]
pub struct Rotation {
    pub data: u8,
    pub basis: Basis3f,
}

/// A scene-graph node. Ported from the `magica::{TransformNode,GroupNode,
/// ShapeNode}` trio + the `Node::Type` enum.
#[derive(Debug, Clone)]
pub enum Node {
    /// `nTRN` — places a child under a layer with a rotation/translation.
    Transform(TransformNode),
    /// `nGRP` — unordered collection of child node ids.
    Group(GroupNode),
    /// `nSHP` — binds a model id.
    Shape(ShapeNode),
}

/// Common header fields shared by every node variant. Ported from the C++
/// `Node` base.
#[derive(Debug, Clone, Default)]
pub struct NodeCommon {
    pub id: i32,
    /// Free-form `_key`/`value` string pairs from the chunk.
    pub attributes: HashMap<String, String>,
}

impl NodeCommon {
    /// Filched from every chunk handler: read node id + attributes.
    pub(crate) fn take_attributes(&mut self, attrs: HashMap<String, String>) {
        self.attributes = attrs;
    }
}

/// `nTRN` node. Ported from `magica::TransformNode`.
#[derive(Debug, Clone, Default)]
pub struct TransformNode {
    pub common: NodeCommon,
    pub child_node_id: i32,
    pub layer_id: i32,
    /// Pivot position in OpenGL coords (see [`parser::magica_to_opengl`]).
    pub position: Vector3i,
    pub rotation: Rotation,
    pub name: String,
    pub hidden: bool,
}

/// `nGRP` node. Ported from `magica::GroupNode`.
#[derive(Debug, Clone, Default)]
pub struct GroupNode {
    pub common: NodeCommon,
    pub child_node_ids: Vec<i32>,
}

/// `nSHP` node. Ported from `magica::ShapeNode`.
#[derive(Debug, Clone, Default)]
pub struct ShapeNode {
    pub common: NodeCommon,
    pub model_id: i32,
    pub model_attributes: HashMap<String, String>,
}

/// `LAYR` chunk. Ported from `magica::Layer`.
#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub id: i32,
    pub attributes: HashMap<String, String>,
    pub name: String,
    pub hidden: bool,
}

/// Material type reported by the `MATL` chunk's `_type` attribute.
/// Ported from `magica::Material::Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaterialType {
    #[default]
    Diffuse,
    Metal,
    Glass,
    Emit,
}

/// `MATL` chunk. Ported from `magica::Material`. Unknown attributes from the
/// spec (`_plastic`) are still ignored, matching upstream.
#[derive(Debug, Clone, Default)]
pub struct Material {
    pub id: i32,
    #[allow(dead_code)]
    pub r#type: MaterialType,
    pub weight: f32,
    pub roughness: f32,
    pub specular: f32,
    pub ior: f32,
    pub flux: f32,
    pub att: f32,
}

/// The parsed contents of a `.vox` file.
///
/// Ported from `magica::Data`. Ownership follows the C++ shape: models and
/// layers are vectors in chunk order; nodes are keyed by id; materials are
/// keyed by palette index.
///
/// Note: `Default` cannot be derived because `[Color8; 256]` exceeds the
/// 32-element derive limit for arrays; the manual impl seeds the palette with
/// [`crate::format::vox::parser`'s default palette][super::parser] (the same
/// one the C++ static initializer uses).
#[derive(Debug)]
pub struct Data {
    pub models: Vec<Model>,
    pub layers: Vec<Layer>,
    /// `id → node`. Built incrementally as `nTRN`/`nGRP`/`nSHP` chunks arrive.
    pub scene_graph: HashMap<i32, Node>,
    /// `palette_index → material`.
    pub materials: HashMap<i32, Material>,
    /// `-1` when the file has no scene graph.
    pub root_node_id: i32,
    pub palette: [Color8; PALETTE_SIZE],
}

impl Default for Data {
    fn default() -> Self {
        Self {
            models: Vec::new(),
            layers: Vec::new(),
            scene_graph: HashMap::new(),
            materials: HashMap::new(),
            root_node_id: -1,
            palette: super::parser::default_palette(),
        }
    }
}

impl Data {
    /// `Data::clear`. Kept for parity but [`Default`] / reassignment is the
    /// idiomatic Rust way to reset.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// `Data::get_model_count`.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// `Data::get_model`.
    pub fn model(&self, index: usize) -> &Model {
        &self.models[index]
    }

    /// `Data::get_root_node_id`.
    pub fn root_node_id(&self) -> i32 {
        self.root_node_id
    }

    /// `Data::get_node`.
    pub fn node(&self, id: i32) -> &Node {
        self.scene_graph
            .get(&id)
            .expect("vox node id not in scene graph")
    }

    /// `Data::get_layer_count`.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// `Data::get_layer_by_index`.
    pub fn layer(&self, index: usize) -> &Layer {
        &self.layers[index]
    }

    /// `Data::get_material_id_for_palette_index`. Returns the palette index
    /// itself if a material was defined for it, else `-1`.
    pub fn material_id_for_palette_index(&self, palette_index: i32) -> i32 {
        if self.materials.contains_key(&palette_index) {
            palette_index
        } else {
            -1
        }
    }

    /// `Data::get_material_by_id`.
    pub fn material(&self, id: i32) -> &Material {
        self.materials
            .get(&id)
            .expect("vox material id not present")
    }

    /// `Data::get_palette`.
    pub fn palette(&self) -> &[Color8; PALETTE_SIZE] {
        &self.palette
    }
}

impl Node {
    /// Borrow the shared header fields, regardless of variant. Stands in for
    /// the C++ "every node is-a Node" base pointer.
    pub fn common(&self) -> &NodeCommon {
        match self {
            Node::Transform(n) => &n.common,
            Node::Group(n) => &n.common,
            Node::Shape(n) => &n.common,
        }
    }

    /// `Node::id`.
    pub fn id(&self) -> i32 {
        self.common().id
    }
}
