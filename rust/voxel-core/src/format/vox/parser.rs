//! Parser for the MagicaVoxel `.vox` binary format.
//!
//! Ported from `streams/vox/vox_data.cpp`. The C++ reader pulls bytes from a
//! Godot `FileAccess`; here we read from any `&[u8]` source via a small
//! fallible [`Reader`] cursor. `.vox` is little-endian.
//!
//! # Format reference
//! - <https://github.com/ephtracy/voxel-model/blob/master/MagicaVoxel-file-format-vox.txt>
//! - <https://github.com/ephtracy/voxel-model/blob/master/MagicaVoxel-file-format-vox-extension.txt>
//!
//! # Coordinate system
//! MagicaVoxel is Z-up. Internally the loader swaps axes to the engine's
//! convention via [`magica_to_opengl`]:
//!
//! ```text
//!     Z             Y
//!     | Y           | X
//!     |/            |/
//!     o----X        o----Z
//!   MagicaVoxel     engine
//! ```

use std::collections::HashMap;

use crate::format::vox::data::{
    Data, GroupNode, Layer, Material, MaterialType, Model, Node, NodeCommon, Rotation, ShapeNode,
    TransformNode, MAX_MODEL_SIZE, PALETTE_SIZE,
};
use crate::math::{Basis3f, Color8, Vector3f, Vector3i};
use crate::streams::DecodeLimits;

/// Parse error. Mirrors the `Error` codes returned by the C++ `_load_from_file`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoxError {
    /// `'VOX '` magic missing or unsupported version.
    BadHeader,
    /// Truncated chunk / read past end-of-file.
    UnexpectedEof,
    /// A field had a value outside the documented range (negative count,
    /// oversized model, duplicate id, …).
    InvalidData(String),
    /// A referential check failed (child/layer/model id points nowhere,
    /// no single root node, …).
    BadSceneGraph(String),
}

impl std::fmt::Display for VoxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VoxError::BadHeader => write!(f, "bad VOX header (magic or version)"),
            VoxError::UnexpectedEof => write!(f, "unexpected end of file"),
            VoxError::InvalidData(m) => write!(f, "invalid vox data: {m}"),
            VoxError::BadSceneGraph(m) => write!(f, "bad vox scene graph: {m}"),
        }
    }
}

impl std::error::Error for VoxError {}

type Result<T> = std::result::Result<T, VoxError>;

/// Little-endian byte cursor over a `&[u8]`. Stands in for Godot's
/// `FileAccess`; reads are bounds-checked and return [`VoxError::UnexpectedEof`]
/// rather than panicking.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn position(&self) -> usize {
        self.pos
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Pull `n` bytes, advancing the cursor. Returns `UnexpectedEof` short.
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(VoxError::UnexpectedEof)?;
        if end > self.data.len() {
            return Err(VoxError::UnexpectedEof);
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(b[0] as u32 | ((b[1] as u32) << 8) | ((b[2] as u32) << 16) | ((b[3] as u32) << 24))
    }

    /// Read 4 bytes as a tag like `b"SIZE"`.
    fn tag(&mut self) -> Result<[u8; 4]> {
        let b = self.take(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }
}

/// `magica_to_opengl` — axis swap from MagicaVoxel's Z-up system.
pub(crate) fn magica_to_opengl(src: Vector3i) -> Vector3i {
    Vector3i::new(src.y, src.z, src.x)
}

/// Transpose a 3×3 matrix given by rows `sx/sy/sz` into `dx/dy/dz`.
/// Matches the C++ `transpose` free function.
fn transpose(sx: Vector3i, sy: Vector3i, sz: Vector3i) -> (Vector3i, Vector3i, Vector3i) {
    let dx = Vector3i::new(sx.x, sy.x, sz.x);
    let dy = Vector3i::new(sx.y, sy.y, sz.y);
    let dz = Vector3i::new(sx.z, sy.z, sz.z);
    (dx, dy, dz)
}

/// Decode a packed rotation byte into a basis.
///
/// Bits 0-1: column index of the non-zero entry in row X.
/// Bits 2-3: column index of the non-zero entry in row Y.
/// Bits 4-6: sign bits for X/Y/Z rows.
/// Ported from `parse_basis`.
// `>> 0` inside `parse_basis` is kept verbatim to mirror the C++ bit-field
// comments even though it's a runtime no-op.
#[allow(clippy::identity_op)]
pub(crate) fn parse_basis(data: u8) -> Basis3f {
    let xi = (data >> 0) & 0x03;
    let yi = (data >> 2) & 0x03;

    // Spec only documents `xi,yi ∈ {0,1,2}` with `xi≠yi` (96 valid bytes).
    // The C++ decoder reads `occupied[xi]`/`occupied[yi]` unchecked; out-of-range
    // or duplicate indices are UB there. We guard against it by falling back to
    // an identity basis for malformed bytes, which keeps the parser panic-free
    // on real-world files that happen to carry a bogus `_r` value.
    if xi >= 3 || yi >= 3 || xi == yi {
        return Basis3f::from_axes(
            Vector3f::new(1.0, 0.0, 0.0),
            Vector3f::new(0.0, 1.0, 0.0),
            Vector3f::new(0.0, 0.0, 1.0),
        );
    }

    // The Z row's non-zero column is whichever of {0,1,2} X and Y didn't take.
    let mut occupied = [false; 3];
    occupied[xi as usize] = true;
    occupied[yi as usize] = true;
    let zi = if !occupied[0] {
        0
    } else if !occupied[1] {
        1
    } else {
        2
    };

    let x_sign = if (data >> 4) & 0x01 == 0 { 1 } else { -1 };
    let y_sign = if (data >> 5) & 0x01 == 0 { 1 } else { -1 };
    let z_sign = if (data >> 6) & 0x01 == 0 { 1 } else { -1 };

    let mut x = Vector3i::splat(0);
    let mut y = Vector3i::splat(0);
    let mut z = Vector3i::splat(0);
    x[xi as usize] = x_sign;
    y[yi as usize] = y_sign;
    z[zi as usize] = z_sign;

    // The C++ comment notes the next steps took some figuring out; we mirror
    // them exactly to keep byte-for-byte basis parity.
    let (magica_x, magica_y, magica_z) = transpose(x, y, z);
    let magica_x = magica_to_opengl(magica_x);
    let magica_y = magica_to_opengl(magica_y);
    let magica_z = magica_to_opengl(magica_z);
    z = magica_x;
    x = magica_y;
    y = magica_z;

    // `Basis::set(x.x, y.x, z.x, x.y, y.y, z.y, x.z, y.z, z.z)` — column-major
    // from the (now reassigned) row vectors.
    Basis3f::from_axes(
        Vector3f::new(x.x as f32, x.y as f32, x.z as f32),
        Vector3f::new(y.x as f32, y.y as f32, y.z as f32),
        Vector3f::new(z.x as f32, z.y as f32, z.z as f32),
    )
}

/// Parse a length-prefixed UTF-8 string. Matches `parse_string`. The C++
/// version caps the length at 4096 and rejects negative sizes.
fn parse_string(r: &mut Reader<'_>, limits: DecodeLimits) -> Result<String> {
    let size = i32_from_u32(r.u32()?);
    if size < 0 {
        return Err(VoxError::InvalidData(format!(
            "string length out of range: {size}"
        )));
    }
    limits
        .check_string_bytes(size as usize)
        .map_err(|e| VoxError::InvalidData(e.to_string()))?;
    let bytes = r.take(size as usize)?;
    std::str::from_utf8(bytes)
        .map(|s| s.to_owned())
        .map_err(|_| VoxError::InvalidData("string is not valid UTF-8".into()))
}

/// Parse a `{key,value}` dictionary. Matches `parse_dictionary` (≤256 entries).
fn parse_dictionary(r: &mut Reader<'_>, limits: DecodeLimits) -> Result<HashMap<String, String>> {
    let item_count = i32_from_u32(r.u32()?);
    if !(0..=256).contains(&item_count) {
        return Err(VoxError::InvalidData(format!(
            "dictionary size out of range: {item_count}"
        )));
    }
    let mut dict = HashMap::with_capacity(item_count as usize);
    for _ in 0..item_count {
        let key = parse_string(r, limits)?;
        let value = parse_string(r, limits)?;
        dict.insert(key, value);
    }
    Ok(dict)
}

/// Common header for every node chunk: id + attributes. Matches
/// `parse_node_common_header`. Returns the parsed common fields and rejects
/// duplicate node ids.
fn parse_node_common_header(
    r: &mut Reader<'_>,
    scene_graph: &HashMap<i32, Node>,
    limits: DecodeLimits,
) -> Result<NodeCommon> {
    let node_id = i32_from_u32(r.u32()?);
    if scene_graph.contains_key(&node_id) {
        return Err(VoxError::InvalidData(format!(
            "node with id {node_id} already exists"
        )));
    }
    let attributes = parse_dictionary(r, limits)?;
    Ok(NodeCommon {
        id: node_id,
        attributes,
    })
}

/// `Data::load_from_file` — the public entry point. `bytes` is the raw file
/// contents (e.g. read with `std::fs::read`).
pub fn parse(bytes: &[u8]) -> Result<Data> {
    parse_with_limits(bytes, DecodeLimits::default())
}

/// `Data::load_from_file` with explicit allocation limits.
pub fn parse_with_limits(bytes: &[u8], limits: DecodeLimits) -> Result<Data> {
    let mut r = Reader::new(bytes);
    // `Data::default` seeds the palette with the documented MagicaVoxel
    // default; an `RGBA` chunk overrides entries 1..255 (index 0 stays
    // transparent).
    let mut data = Data::default();

    let mut last_size = Vector3i::default();
    let mut total_dense_voxels = 0u64;
    let mut scene_node_count = 0usize;

    // --- file header -------------------------------------------------------
    let magic = r.tag()?;
    if &magic != b"VOX " {
        return Err(VoxError::BadHeader);
    }
    let version = r.u32()?;
    // Spec only documents v150; v200 appeared in the wild without spec changes
    // our loader cares about, so we accept both — matching upstream.
    if version != 150 && version != 200 {
        return Err(VoxError::BadHeader);
    }

    let file_length = r.len();

    // --- chunk stream ------------------------------------------------------
    while r.position() < file_length {
        let chunk_id = r.tag()?;
        let chunk_size = r.u32()? as usize;
        // `child_chunks_size` — unused, matches the C++ `f.get_32()` discard.
        let _children_size = r.u32()?;

        let chunk_start = r.position();

        if &chunk_id == b"SIZE" {
            let size = Vector3i::new(
                i32_from_u32(r.u32()?),
                i32_from_u32(r.u32()?),
                i32_from_u32(r.u32()?),
            );
            if !(0..=MAX_MODEL_SIZE).contains(&size.x)
                || !(0..=MAX_MODEL_SIZE).contains(&size.y)
                || !(0..=MAX_MODEL_SIZE).contains(&size.z)
            {
                return Err(VoxError::InvalidData(format!(
                    "model dimension must be in 0..={MAX_MODEL_SIZE}: {size:?}"
                )));
            }
            last_size = magica_to_opengl(size);
        } else if &chunk_id == b"XYZI" {
            let num_voxels = r.u32()?;
            limits
                .check_vox_models(data.models.len() + 1)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            let dense_voxels = last_size.volume_u64();
            if u64::from(num_voxels) > dense_voxels {
                return Err(VoxError::InvalidData(format!(
                    "XYZI voxel count {num_voxels} exceeds model volume {dense_voxels}"
                )));
            }
            total_dense_voxels = total_dense_voxels
                .checked_add(dense_voxels)
                .ok_or_else(|| VoxError::InvalidData("vox total voxel count overflow".into()))?;
            limits
                .check_vox_total_voxels(total_dense_voxels)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            let color_index_len = usize::try_from(dense_voxels)
                .map_err(|_| VoxError::InvalidData("model color index length overflow".into()))?;
            limits
                .check_bytes("vox model color indexes", color_index_len)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            let mut color_indexes = Vec::new();
            color_indexes.try_reserve(color_index_len).map_err(|_| {
                VoxError::InvalidData(format!(
                    "model color index allocation failed for {color_index_len} bytes"
                ))
            })?;
            color_indexes.resize(color_index_len, 0u8);
            let mut model = Model {
                size: last_size,
                color_indexes,
            };
            for _ in 0..num_voxels {
                let pos = Vector3i::new(r.u8()? as i32, r.u8()? as i32, r.u8()? as i32);
                let c = r.u8()?;
                let pos = magica_to_opengl(pos);
                if pos.x < 0
                    || pos.x >= model.size.x
                    || pos.y < 0
                    || pos.y >= model.size.y
                    || pos.z < 0
                    || pos.z >= model.size.z
                {
                    return Err(VoxError::InvalidData(format!(
                        "voxel position {pos:?} out of model bounds {:?}",
                        model.size
                    )));
                }
                let idx = pos.zxy_index(model.size) as usize;
                model.color_indexes[idx] = c;
            }
            data.models.push(model);
        } else if &chunk_id == b"RGBA" {
            // Index 0 stays transparent (matches `_palette[0] = {0,0,0,0}`).
            data.palette[0] = Color8::new(0, 0, 0, 0);
            // The chunk documents 255 colors; index 0 is reserved.
            for i in 1..PALETTE_SIZE {
                let rr = r.u8()?;
                let gg = r.u8()?;
                let bb = r.u8()?;
                let aa = r.u8()?;
                data.palette[i] = Color8::new(rr, gg, bb, aa);
            }
            // Trailing reserved u32 (matches `f.get_32()` discard).
            let _ = r.u32()?;
        } else if &chunk_id == b"nTRN" {
            let mut common = parse_node_common_header(&mut r, &data.scene_graph, limits)?;
            let mut node = TransformNode::default();
            if let Some(name) = common.attributes.remove("_name") {
                node.name = name;
            }
            node.hidden = match common.attributes.remove("_hidden") {
                Some(v) => v == "1",
                None => false,
            };
            node.common = common;

            node.child_node_id = i32_from_u32(r.u32()?);
            let reserved = i32_from_u32(r.u32()?);
            if reserved != -1 {
                return Err(VoxError::InvalidData(
                    "nTRN reserved field was not -1".into(),
                ));
            }
            node.layer_id = i32_from_u32(r.u32()?);

            let frame_count = i32_from_u32(r.u32()?);
            if frame_count != 1 {
                return Err(VoxError::InvalidData(format!(
                    "nTRN frame_count must be 1, got {frame_count}"
                )));
            }

            let frame = parse_dictionary(&mut r, limits)?;
            if let Some(t) = frame.get("_t") {
                // Three space-separated integers in text form.
                let coords: Vec<i32> = t.split(' ').filter_map(|s| s.parse().ok()).collect();
                if coords.len() < 3 {
                    return Err(VoxError::InvalidData(format!(
                        "nTRN _t has fewer than 3 coords: {t:?}"
                    )));
                }
                node.position = magica_to_opengl(Vector3i::new(coords[0], coords[1], coords[2]));
            }
            if let Some(rv) = frame.get("_r") {
                let rot_byte = rv.parse::<u32>().unwrap_or(0) as u8;
                node.rotation = Rotation {
                    data: rot_byte,
                    basis: parse_basis(rot_byte),
                };
            }

            scene_node_count += 1;
            limits
                .check_vox_nodes(scene_node_count)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            data.scene_graph
                .insert(node.common.id, Node::Transform(node));
        } else if &chunk_id == b"nGRP" {
            let common = parse_node_common_header(&mut r, &data.scene_graph, limits)?;
            let child_count = r.u32()?;
            if child_count > 65536 {
                return Err(VoxError::InvalidData(format!(
                    "nGRP child_count too large: {child_count}"
                )));
            }
            let mut child_node_ids = Vec::with_capacity(child_count as usize);
            for _ in 0..child_count {
                child_node_ids.push(i32_from_u32(r.u32()?));
            }
            scene_node_count += 1;
            limits
                .check_vox_nodes(scene_node_count)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            data.scene_graph.insert(
                common.id,
                Node::Group(GroupNode {
                    common,
                    child_node_ids,
                }),
            );
        } else if &chunk_id == b"nSHP" {
            let mut common = parse_node_common_header(&mut r, &data.scene_graph, limits)?;
            let model_count = r.u32()?;
            if model_count != 1 {
                return Err(VoxError::InvalidData(format!(
                    "nSHP model_count must be 1, got {model_count}"
                )));
            }
            let model_id = i32_from_u32(r.u32()?);
            if !(0..=65536).contains(&model_id) {
                return Err(VoxError::InvalidData(format!(
                    "nSHP model_id out of range: {model_id}"
                )));
            }
            let model_attributes = parse_dictionary(&mut r, limits)?;
            common.take_attributes(Default::default()); // common keeps its own attrs
            scene_node_count += 1;
            limits
                .check_vox_nodes(scene_node_count)
                .map_err(|e| VoxError::InvalidData(e.to_string()))?;
            data.scene_graph.insert(
                common.id,
                Node::Shape(ShapeNode {
                    common,
                    model_id,
                    model_attributes,
                }),
            );
        } else if &chunk_id == b"LAYR" {
            let layer_id = i32_from_u32(r.u32()?);
            if data.layers.iter().any(|l| l.id == layer_id) {
                return Err(VoxError::InvalidData(format!(
                    "layer with id {layer_id} already exists"
                )));
            }
            let mut attributes = parse_dictionary(&mut r, limits)?;
            let name = attributes.remove("_name").unwrap_or_default();
            let hidden = match attributes.remove("_hidden") {
                Some(v) => v == "1",
                None => false,
            };
            let reserved = i32_from_u32(r.u32()?);
            if reserved != -1 {
                return Err(VoxError::InvalidData(
                    "LAYR reserved field was not -1".into(),
                ));
            }
            data.layers.push(Layer {
                id: layer_id,
                attributes,
                name,
                hidden,
            });
        } else if &chunk_id == b"MATL" {
            let material_id = i32_from_u32(r.u32()?);
            if !(0..=PALETTE_SIZE as i32).contains(&material_id) {
                return Err(VoxError::InvalidData(format!(
                    "MATL id out of range: {material_id}"
                )));
            }
            if data.materials.contains_key(&material_id) {
                return Err(VoxError::InvalidData(format!(
                    "material id {material_id} already exists"
                )));
            }
            let attributes = parse_dictionary(&mut r, limits)?;
            let mut material = Material {
                id: material_id,
                ..Default::default()
            };
            material.r#type = match attributes.get("_type").map(String::as_str) {
                Some("_diffuse") => MaterialType::Diffuse,
                Some("_metal") => MaterialType::Metal,
                Some("_glass") => MaterialType::Glass,
                Some("_emit") => MaterialType::Emit,
                _ => MaterialType::Diffuse,
            };
            material.weight = attributes
                .get("_weight")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0);
            material.roughness = attributes
                .get("_rough")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0);
            material.specular = attributes
                .get("_spec")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0);
            material.ior = attributes
                .get("_ior")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0);
            material.att = attributes
                .get("_att")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0);
            material.flux = attributes
                .get("_flux")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0);

            data.materials.insert(material_id, material);
        } else {
            // Unknown chunk — skip its payload, mirroring `f.seek(pos + chunk_size)`.
            let next = chunk_start
                .checked_add(chunk_size)
                .ok_or(VoxError::UnexpectedEof)?;
            if next > r.len() {
                return Err(VoxError::UnexpectedEof);
            }
            r.seek(next);
        }

        // The format doesn't always align chunk boundaries with `chunk_size`
        // (children sit in the trailing `child_chunks_size` region); the C++
        // loop just re-reads the next tag, so we do the same — no forced seek.
    }

    data.root_node_id = validate_scene_graph(&data)?;
    Ok(data)
}

/// Final referential-validation pass: every referenced child/layer/model must
/// exist, and exactly one node must be unreferenced (the root). Returns the
/// resolved root node id (or `-1` when the file has no scene graph).
fn validate_scene_graph(data: &Data) -> Result<i32> {
    let mut referenced: std::collections::HashSet<i32> = std::collections::HashSet::new();

    for node in data.scene_graph.values() {
        match node {
            Node::Transform(t) => {
                let child_id = t.child_node_id;
                if !data.scene_graph.contains_key(&child_id) {
                    return Err(VoxError::BadSceneGraph(format!(
                        "child node {child_id} does not exist"
                    )));
                }
                referenced.insert(child_id);

                let layer_id = t.layer_id;
                if layer_id != -1 && !data.layers.iter().any(|l| l.id == layer_id) {
                    return Err(VoxError::BadSceneGraph(format!(
                        "layer {layer_id} does not exist"
                    )));
                }
            }
            Node::Group(g) => {
                for &child_id in &g.child_node_ids {
                    if !data.scene_graph.contains_key(&child_id) {
                        return Err(VoxError::BadSceneGraph(format!(
                            "child node {child_id} does not exist"
                        )));
                    }
                    referenced.insert(child_id);
                }
            }
            Node::Shape(s) => {
                let model_id = s.model_id;
                if model_id < 0 || model_id as usize >= data.models.len() {
                    return Err(VoxError::BadSceneGraph(format!(
                        "model {model_id} does not exist"
                    )));
                }
            }
        }
    }

    // The single unreferenced node is the root. (A cycle would leave none.)
    let mut root_node_id = -1;
    for &id in data.scene_graph.keys() {
        if referenced.contains(&id) {
            continue;
        }
        if root_node_id != -1 {
            return Err(VoxError::BadSceneGraph(
                "more than one root node was found".into(),
            ));
        }
        root_node_id = id;
    }

    if !data.scene_graph.is_empty() && root_node_id == -1 {
        return Err(VoxError::BadSceneGraph(
            "scene graph has no root node (possible cycle)".into(),
        ));
    }

    Ok(root_node_id)
}

/// Reinterpret a `u32` as a signed `i32` (two's complement), matching the C++
/// implicit conversion from `f.get_32()` into `int`.
pub(super) fn i32_from_u32(v: u32) -> i32 {
    v as i32
}

// The default MagicaVoxel palette. Each entry is packed `0xRRGGBBAA`, but the
// loader stores it as a Rust `Color8 { r, g, b, a }`. Ported verbatim from
// `g_default_palette` in vox_data.cpp.
const DEFAULT_PALETTE_PACKED: [u32; PALETTE_SIZE] = [
    0x00000000, 0xffffffff, 0xffccffff, 0xff99ffff, 0xff66ffff, 0xff33ffff, 0xff00ffff, 0xffffccff,
    0xffccccff, 0xff99ccff, 0xff66ccff, 0xff33ccff, 0xff00ccff, 0xffff99ff, 0xffcc99ff, 0xff9999ff,
    0xff6699ff, 0xff3399ff, 0xff0099ff, 0xffff66ff, 0xffcc66ff, 0xff9966ff, 0xff6666ff, 0xff3366ff,
    0xff0066ff, 0xffff33ff, 0xffcc33ff, 0xff9933ff, 0xff6633ff, 0xff3333ff, 0xff0033ff, 0xffff00ff,
    0xffcc00ff, 0xff9900ff, 0xff6600ff, 0xff3300ff, 0xff0000ff, 0xffffffcc, 0xffccffcc, 0xff99ffcc,
    0xff66ffcc, 0xff33ffcc, 0xff00ffcc, 0xffffcccc, 0xffcccccc, 0xff99cccc, 0xff66cccc, 0xff33cccc,
    0xff00cccc, 0xffff99cc, 0xffcc99cc, 0xff9999cc, 0xff6699cc, 0xff3399cc, 0xff0099cc, 0xffff66cc,
    0xffcc66cc, 0xff9966cc, 0xff6666cc, 0xff3366cc, 0xff0066cc, 0xffff33cc, 0xffcc33cc, 0xff9933cc,
    0xff6633cc, 0xff3333cc, 0xff0033cc, 0xffff00cc, 0xffcc00cc, 0xff9900cc, 0xff6600cc, 0xff3300cc,
    0xff0000cc, 0xffffff99, 0xffccff99, 0xff99ff99, 0xff66ff99, 0xff33ff99, 0xff00ff99, 0xffffcc99,
    0xffcccc99, 0xff99cc99, 0xff66cc99, 0xff33cc99, 0xff00cc99, 0xffff9999, 0xffcc9999, 0xff999999,
    0xff669999, 0xff339999, 0xff009999, 0xffff6699, 0xffcc6699, 0xff996699, 0xff666699, 0xff336699,
    0xff006699, 0xffff3399, 0xffcc3399, 0xff993399, 0xff663399, 0xff333399, 0xff003399, 0xffff0099,
    0xffcc0099, 0xff990099, 0xff660099, 0xff330099, 0xff000099, 0xffffff66, 0xffccff66, 0xff99ff66,
    0xff66ff66, 0xff33ff66, 0xff00ff66, 0xffffcc66, 0xffcccc66, 0xff99cc66, 0xff66cc66, 0xff33cc66,
    0xff00cc66, 0xffff9966, 0xffcc9966, 0xff999966, 0xff669966, 0xff339966, 0xff009966, 0xffff6666,
    0xffcc6666, 0xff996666, 0xff666666, 0xff336666, 0xff006666, 0xffff3366, 0xffcc3366, 0xff993366,
    0xff663366, 0xff333366, 0xff003366, 0xffff0066, 0xffcc0066, 0xff990066, 0xff660066, 0xff330066,
    0xff000066, 0xffffff33, 0xffccff33, 0xff99ff33, 0xff66ff33, 0xff33ff33, 0xff00ff33, 0xffffcc33,
    0xffcccc33, 0xff99cc33, 0xff66cc33, 0xff33cc33, 0xff00cc33, 0xffff9933, 0xffcc9933, 0xff999933,
    0xff669933, 0xff339933, 0xff009933, 0xffff6633, 0xffcc6633, 0xff996633, 0xff666633, 0xff336633,
    0xff006633, 0xffff3333, 0xffcc3333, 0xff993333, 0xff663333, 0xff333333, 0xff003333, 0xffff0033,
    0xffcc0033, 0xff990033, 0xff660033, 0xff330033, 0xff000033, 0xffffff00, 0xffccff00, 0xff99ff00,
    0xff66ff00, 0xff33ff00, 0xff00ff00, 0xffffcc00, 0xffcccc00, 0xff99cc00, 0xff66cc00, 0xff33cc00,
    0xff00cc00, 0xffff9900, 0xffcc9900, 0xff999900, 0xff669900, 0xff339900, 0xff009900, 0xffff6600,
    0xffcc6600, 0xff996600, 0xff666600, 0xff336600, 0xff006600, 0xffff3300, 0xffcc3300, 0xff993300,
    0xff663300, 0xff333300, 0xff003300, 0xffff0000, 0xffcc0000, 0xff990000, 0xff660000, 0xff330000,
    0xff0000ee, 0xff0000dd, 0xff0000bb, 0xff0000aa, 0xff000088, 0xff000077, 0xff000055, 0xff000044,
    0xff000022, 0xff000011, 0xff00ee00, 0xff00dd00, 0xff00bb00, 0xff00aa00, 0xff008800, 0xff007700,
    0xff005500, 0xff004400, 0xff002200, 0xff001100, 0xffee0000, 0xffdd0000, 0xffbb0000, 0xffaa0000,
    0xff880000, 0xff770000, 0xff550000, 0xff440000, 0xff220000, 0xff110000, 0xffeeeeee, 0xffdddddd,
    0xffbbbbbb, 0xffaaaaaa, 0xff888888, 0xff777777, 0xff555555, 0xff444444, 0xff222222, 0xff111111,
];

/// Build the default 256-entry palette as `Color8`. Matches the C++ static
/// `g_default_palette` initializer.
pub(crate) fn default_palette() -> [Color8; PALETTE_SIZE] {
    let mut palette = [Color8::new(0, 0, 0, 0); PALETTE_SIZE];
    for (i, &packed) in DEFAULT_PALETTE_PACKED.iter().enumerate() {
        // Packed as 0xRRGGBBAA.
        palette[i] = Color8::new(
            ((packed >> 24) & 0xff) as u8,
            ((packed >> 16) & 0xff) as u8,
            ((packed >> 8) & 0xff) as u8,
            (packed & 0xff) as u8,
        );
    }
    palette
}
