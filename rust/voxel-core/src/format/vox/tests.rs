//! Tests for the MagicaVoxel `.vox` parser.
//!
//! Covered:
//! - Default palette parity with the C++ `g_default_palette` initializer.
//! - `magica_to_opengl` axis swap.
//! - `parse_basis` for representative rotation bytes (identity, 90°, 180°).
//! - End-to-end parse of a synthetic `.vox` byte stream: header + SIZE + XYZI,
//!   plus a full scene-graph variant (nTRN/nGRP/nSHP/LAYR/MATL/RGBA).
//! - Error paths: bad magic, unsupported version, truncation, oversized model,
//!   duplicate node id, dangling child reference.

#![cfg(test)]

use super::data::{Data, MaterialType, Node, PALETTE_SIZE};
use super::parser::{
    default_palette, i32_from_u32, magica_to_opengl, parse, parse_basis, parse_with_limits,
    VoxError,
};
use crate::math::{Color8, Vector3i};

// ---- helper: little-endian encoders for building synthetic .vox streams ----

fn u32_le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn i32_le(v: i32) -> [u8; 4] {
    (v as u32).to_le_bytes()
}

/// Build a `.vox` byte stream from a list of (tag, payload) chunks. Inserts the
/// `VOX ` magic + version 150 and the per-chunk `(size, children_size)` header
/// (both zero — the parser doesn't trust them for traversal).
fn vox_file(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    // Note: callers pass `b"SIZE"` (already `&[u8; 4]`), not `*b"SIZE"`.
    let mut out = Vec::new();
    out.extend_from_slice(b"VOX ");
    out.extend_from_slice(&u32_le(150));
    for (tag, payload) in chunks {
        // `tag: &&[u8;4]` — deref once for `extend_from_slice`.
        out.extend_from_slice(*tag);
        out.extend_from_slice(&u32_le(payload.len() as u32));
        out.extend_from_slice(&u32_le(0)); // children_size
        out.extend_from_slice(payload);
    }
    out
}

// ===========================================================================
// Default palette
// ===========================================================================

#[test]
fn default_palette_has_256_entries() {
    let p = default_palette();
    assert_eq!(p.len(), PALETTE_SIZE);
}

#[test]
fn default_palette_index_0_is_transparent() {
    // Matches `_palette[0] = Color8{0,0,0,0}` reservation.
    let p = default_palette();
    assert_eq!(p[0], Color8::new(0, 0, 0, 0));
}

#[test]
fn default_palette_index_1_is_white() {
    // 0xffffffff → R=0xff,G=0xff,B=0xff,A=0xff.
    let p = default_palette();
    assert_eq!(p[1], Color8::new(0xff, 0xff, 0xff, 0xff));
}

#[test]
fn default_palette_matches_packed_source() {
    // Spot-check a few entries against the 0xRRGGBBAA packing in vox_data.cpp.
    let p = default_palette();
    // Index 7: 0xffffccff → R=ff G=ff B=cc A=ff
    assert_eq!(p[7], Color8::new(0xff, 0xff, 0xcc, 0xff));
    // Index 36: 0xff0000ff → pure red, full alpha
    assert_eq!(p[36], Color8::new(0xff, 0x00, 0x00, 0xff));
    // Index 255: 0xff111111 → near-black
    assert_eq!(p[255], Color8::new(0xff, 0x11, 0x11, 0x11));
}

// ===========================================================================
// magica_to_opengl
// ===========================================================================

#[test]
fn magica_to_opengl_swaps_axes() {
    // (x,y,z) → (y,z,x) per the diagram.
    assert_eq!(
        magica_to_opengl(Vector3i::new(1, 2, 3)),
        Vector3i::new(2, 3, 1)
    );
    assert_eq!(
        magica_to_opengl(Vector3i::new(10, 0, 0)),
        Vector3i::new(0, 0, 10)
    );
    assert_eq!(
        magica_to_opengl(Vector3i::new(0, 0, 0)),
        Vector3i::new(0, 0, 0)
    );
}

// ===========================================================================
// parse_basis
// ===========================================================================

#[test]
fn parse_basis_identity_is_canonical() {
    // Valid rotation byte 1: xi=1, yi=0, zi=2 (deduced), all signs positive.
    // The basis is non-trivial due to the axis swap but must be axis-aligned.
    let b = parse_basis(1);
    // Each column of the resulting basis must have exactly one ±1 and two 0s.
    for col in 0..3 {
        let c = b.get_column(col);
        let nonzero = [c.x, c.y, c.z].iter().filter(|&&v| v != 0.0).count();
        assert_eq!(
            nonzero, 1,
            "col {col} of basis is not axis-aligned: ({},{},{})",
            c.x, c.y, c.z
        );
    }
}

/// All valid MagicaVoxel rotation bytes: `xi,yi ∈ {0,1,2}`, `xi≠yi`, plus
/// independent sign bits. 96 combinations total.
fn valid_rotation_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    for data in 0u8..=255 {
        let xi = data & 0x03;
        let yi = (data >> 2) & 0x03;
        if xi < 3 && yi < 3 && xi != yi {
            out.push(data);
        }
    }
    out
}

#[test]
fn parse_basis_is_orthonormal() {
    // Every valid rotation byte should produce an orthonormal basis.
    for data in valid_rotation_bytes() {
        let b = parse_basis(data);
        let x = b.get_column(0);
        let y = b.get_column(1);
        let z = b.get_column(2);
        let dot_xy = x.x * y.x + x.y * y.y + x.z * y.z;
        let dot_xz = x.x * z.x + x.y * z.y + x.z * z.z;
        let dot_yz = y.x * z.x + y.y * z.y + y.z * z.z;
        assert!(
            dot_xy.abs() < 1e-5 && dot_xz.abs() < 1e-5 && dot_yz.abs() < 1e-5,
            "rotation byte {data}: basis is not orthogonal"
        );
        let xn = (x.x * x.x + x.y * x.y + x.z * x.z).abs();
        let yn = (y.x * y.x + y.y * y.y + y.z * y.z).abs();
        let zn = (z.x * z.x + z.y * z.y + z.z * z.z).abs();
        assert!(
            (xn - 1.0).abs() < 1e-5,
            "rotation byte {data}: x not unit ({xn})"
        );
        assert!(
            (yn - 1.0).abs() < 1e-5,
            "rotation byte {data}: y not unit ({yn})"
        );
        assert!(
            (zn - 1.0).abs() < 1e-5,
            "rotation byte {data}: z not unit ({zn})"
        );
    }
}

#[test]
fn parse_basis_invalid_byte_falls_back_to_identity() {
    // Bytes where xi≥3, yi≥3, or xi==yi are out of spec; the decoder must not
    // panic and must return a usable basis. Identity is the documented fallback.
    for &bad in &[0u8, 3u8, 4u8, 5u8, 255u8] {
        // 0: xi=yi=0 (collision); 3: xi=3 (OOB); 5: xi=1,yi=1 (collision).
        let b = parse_basis(bad);
        let x = b.get_column(0);
        let y = b.get_column(1);
        let z = b.get_column(2);
        // Identity columns.
        assert!((x.x - 1.0).abs() < 1e-6, "byte {bad}: x not (1,0,0)");
        assert!((y.y - 1.0).abs() < 1e-6, "byte {bad}: y not (0,1,0)");
        assert!((z.z - 1.0).abs() < 1e-6, "byte {bad}: z not (0,0,1)");
    }
}

// ===========================================================================
// i32_from_u32
// ===========================================================================

#[test]
fn i32_from_u32_handles_negative_via_twos_complement() {
    assert_eq!(i32_from_u32(0), 0);
    assert_eq!(i32_from_u32(1), 1);
    assert_eq!(i32_from_u32(u32::MAX), -1); // 0xffffffff → -1
    assert_eq!(i32_from_u32(0xfffffffe), -2);
}

// ===========================================================================
// End-to-end: minimal model
// ===========================================================================

#[test]
fn parse_minimal_model() {
    // One SIZE chunk + one XYZI chunk with a single voxel.
    let mut size_payload = Vec::new();
    size_payload.extend_from_slice(&u32_le(2)); // x
    size_payload.extend_from_slice(&u32_le(3)); // y
    size_payload.extend_from_slice(&u32_le(4)); // z

    let mut xyzi_payload = Vec::new();
    xyzi_payload.extend_from_slice(&u32_le(1)); // num_voxels
                                                // voxel at (0,0,0) with color index 5
    xyzi_payload.extend_from_slice(&[0, 0, 0, 5]);

    let bytes = vox_file(&[(b"SIZE", size_payload), (b"XYZI", xyzi_payload)]);

    let data = parse(&bytes).expect("minimal model should parse");
    assert_eq!(data.model_count(), 1);
    let model = data.model(0);
    // magica_to_opengl((2,3,4)) = (3,4,2)
    assert_eq!(model.size, Vector3i::new(3, 4, 2));
    assert_eq!(model.color_indexes.len(), 3 * 4 * 2);

    // Voxel at magica (0,0,0) → opengl (0,0,0); zxy_index((0,0,0),(3,4,2)) = 0.
    assert_eq!(model.color_indexes[0], 5);
    // All other voxels stay 0 (vec! initialised).
    assert_eq!(model.color_indexes.iter().filter(|&&c| c == 5).count(), 1);
}

#[test]
fn parse_default_palette_loaded_implicitly() {
    // No RGBA chunk → palette must be the documented default.
    let mut size_payload = Vec::new();
    size_payload.extend_from_slice(&u32_le(1));
    size_payload.extend_from_slice(&u32_le(1));
    size_payload.extend_from_slice(&u32_le(1));
    let mut xyzi_payload = Vec::new();
    xyzi_payload.extend_from_slice(&u32_le(0));
    let bytes = vox_file(&[(b"SIZE", size_payload), (b"XYZI", xyzi_payload)]);

    let data = parse(&bytes).unwrap();
    assert_eq!(data.palette()[1], Color8::new(0xff, 0xff, 0xff, 0xff));
    assert_eq!(data.palette()[0], Color8::new(0, 0, 0, 0));
}

#[test]
fn parse_with_limits_rejects_dense_model_allocation_over_limit() {
    let mut size = Vec::new();
    size.extend_from_slice(&u32_le(16));
    size.extend_from_slice(&u32_le(16));
    size.extend_from_slice(&u32_le(16));
    let mut xyzi = Vec::new();
    xyzi.extend_from_slice(&u32_le(0));
    let bytes = vox_file(&[(b"SIZE", size), (b"XYZI", xyzi)]);
    let limits = crate::streams::DecodeLimits {
        max_vox_total_voxels: 16,
        ..crate::streams::DecodeLimits::default()
    };

    match parse_with_limits(&bytes, limits).unwrap_err() {
        VoxError::InvalidData(message) => assert!(message.contains("vox total voxels")),
        other => panic!("expected InvalidData, got {other:?}"),
    }
}

#[test]
fn parse_with_limits_rejects_too_many_models() {
    let mut chunks = Vec::new();
    for _ in 0..2 {
        let mut size = Vec::new();
        size.extend_from_slice(&u32_le(1));
        size.extend_from_slice(&u32_le(1));
        size.extend_from_slice(&u32_le(1));
        let mut xyzi = Vec::new();
        xyzi.extend_from_slice(&u32_le(0));
        chunks.push((b"SIZE", size));
        chunks.push((b"XYZI", xyzi));
    }
    let bytes = vox_file(&chunks);
    let limits = crate::streams::DecodeLimits {
        max_vox_models: 1,
        ..crate::streams::DecodeLimits::default()
    };

    match parse_with_limits(&bytes, limits).unwrap_err() {
        VoxError::InvalidData(message) => assert!(message.contains("vox models")),
        other => panic!("expected InvalidData, got {other:?}"),
    }
}

// ===========================================================================
// RGBA override
// ===========================================================================

#[test]
fn parse_rgba_overrides_palette_entries() {
    let mut rgba = Vec::new();
    // The chunk writes 255 colors (indices 1..255), each r,g,b,a.
    for i in 1..PALETTE_SIZE {
        let v = i as u8;
        rgba.extend_from_slice(&[v, v, v, 0xff]);
    }
    rgba.extend_from_slice(&u32_le(0)); // trailing reserved u32
    let mut size = Vec::new();
    size.extend_from_slice(&[0u8; 12][..]); // 1x1x1
    let mut xyzi = Vec::new();
    xyzi.extend_from_slice(&u32_le(0));
    let bytes = vox_file(&[(b"SIZE", size), (b"XYZI", xyzi), (b"RGBA", rgba)]);

    let data = parse(&bytes).unwrap();
    // Index 0 stays transparent (RGBA never writes it).
    assert_eq!(data.palette()[0], Color8::new(0, 0, 0, 0));
    // Indices 1..255 are grey (r=g=b=index).
    for i in 1..PALETTE_SIZE {
        let v = i as u8;
        assert_eq!(
            data.palette()[i],
            Color8::new(v, v, v, 0xff),
            "palette[{i}]"
        );
    }
}

// ===========================================================================
// Scene graph
// ===========================================================================

fn dict_payload(items: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&u32_le(items.len() as u32));
    for (k, v) in items {
        out.extend_from_slice(&u32_le(k.len() as u32));
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&u32_le(v.len() as u32));
        out.extend_from_slice(v.as_bytes());
    }
    out
}

#[test]
fn parse_full_scene_graph() {
    // Build: model 0 (1³), then nGRP(root) → nTRN → nSHP, plus LAYR + MATL.
    let mut size = Vec::new();
    size.extend_from_slice(&u32_le(1));
    size.extend_from_slice(&u32_le(1));
    size.extend_from_slice(&u32_le(1));
    let mut xyzi = Vec::new();
    xyzi.extend_from_slice(&u32_le(0)); // empty model
    let mut model_chunks = vec![(b"SIZE", size), (b"XYZI", xyzi)];

    // nGRP (id=0) with one child (id=1).
    let mut ngrp = Vec::new();
    ngrp.extend_from_slice(&u32_le(0)); // node id
    ngrp.extend_from_slice(&dict_payload(&[])); // attributes
    ngrp.extend_from_slice(&u32_le(1)); // child count
    ngrp.extend_from_slice(&u32_le(1)); // child id
    model_chunks.push((b"nGRP", ngrp));

    // nTRN (id=1) referencing child id=2, layer id=0, with _t="1 2 3" and _r=0.
    let mut ntrn = Vec::new();
    ntrn.extend_from_slice(&u32_le(1)); // node id
    ntrn.extend_from_slice(&dict_payload(&[("_name", "x")])); // attributes
    ntrn.extend_from_slice(&u32_le(2)); // child_node_id
    ntrn.extend_from_slice(&i32_le(-1)); // reserved
    ntrn.extend_from_slice(&u32_le(0)); // layer_id
    ntrn.extend_from_slice(&u32_le(1)); // frame_count
                                        // frame dictionary: _t and _r
    ntrn.extend_from_slice(&dict_payload(&[("_t", "1 2 3"), ("_r", "0")]));
    model_chunks.push((b"nTRN", ntrn));

    // nSHP (id=2) referencing model 0.
    let mut nshp = Vec::new();
    nshp.extend_from_slice(&u32_le(2)); // node id
    nshp.extend_from_slice(&dict_payload(&[])); // attributes
    nshp.extend_from_slice(&u32_le(1)); // model_count
    nshp.extend_from_slice(&u32_le(0)); // model_id
    nshp.extend_from_slice(&dict_payload(&[])); // model_attributes
    model_chunks.push((b"nSHP", nshp));

    // LAYR id=0.
    let mut layr = Vec::new();
    layr.extend_from_slice(&u32_le(0)); // layer id
    layr.extend_from_slice(&dict_payload(&[("_name", "layer0")]));
    layr.extend_from_slice(&i32_le(-1)); // reserved
    model_chunks.push((b"LAYR", layr));

    // MATL id=1 (_metal).
    let mut matl = Vec::new();
    matl.extend_from_slice(&u32_le(1)); // material id
    matl.extend_from_slice(&dict_payload(&[
        ("_type", "_metal"),
        ("_weight", "0.5"),
        ("_rough", "0.25"),
    ]));
    model_chunks.push((b"MATL", matl));

    let bytes = vox_file(&model_chunks);
    let data = parse(&bytes).expect("scene graph should parse");

    // Root is the one unreferenced node: id=0 (the group).
    assert_eq!(data.root_node_id(), 0);
    assert_eq!(data.scene_graph.len(), 3);

    // Verify node kinds and fields.
    match data.node(0) {
        Node::Group(g) => assert_eq!(g.child_node_ids, vec![1]),
        other => panic!("expected Group, got {other:?}"),
    }
    match data.node(1) {
        Node::Transform(t) => {
            assert_eq!(t.child_node_id, 2);
            assert_eq!(t.layer_id, 0);
            assert_eq!(t.name, "x");
            // magica_to_opengl((1,2,3)) = (2,3,1)
            assert_eq!(t.position, Vector3i::new(2, 3, 1));
            assert_eq!(t.rotation.data, 0);
        }
        other => panic!("expected Transform, got {other:?}"),
    }
    match data.node(2) {
        Node::Shape(s) => assert_eq!(s.model_id, 0),
        other => panic!("expected Shape, got {other:?}"),
    }

    // Layer + material.
    assert_eq!(data.layer_count(), 1);
    assert_eq!(data.layer(0).name, "layer0");
    assert_eq!(data.material_id_for_palette_index(1), 1);
    let mat = data.material(1);
    assert_eq!(mat.r#type, MaterialType::Metal);
    assert!((mat.weight - 0.5).abs() < 1e-6);
    assert!((mat.roughness - 0.25).abs() < 1e-6);
}

#[test]
fn parse_unknown_chunk_is_skipped() {
    // An unknown chunk must be skipped by seeking past its payload.
    let mut size = Vec::new();
    size.extend_from_slice(&u32_le(1));
    size.extend_from_slice(&u32_le(1));
    size.extend_from_slice(&u32_le(1));
    let mut xyzi = Vec::new();
    xyzi.extend_from_slice(&u32_le(0));
    // Unknown 'XXXX' chunk with 8 bytes of garbage.
    let bytes = vox_file(&[
        (b"SIZE", size),
        (b"XYZI", xyzi),
        (
            b"XXXX",
            vec![0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe],
        ),
    ]);

    let data = parse(&bytes).expect("unknown chunk must not abort parsing");
    assert_eq!(data.model_count(), 1);
}

// ===========================================================================
// Error paths
// ===========================================================================

#[test]
fn parse_rejects_bad_magic() {
    let mut bytes = vec![b'X', b'Y', b'Z', b' '];
    bytes.extend_from_slice(&u32_le(150));
    assert_eq!(parse(&bytes).unwrap_err(), VoxError::BadHeader);
}

#[test]
fn parse_rejects_unsupported_version() {
    let mut bytes = vec![b'V', b'O', b'X', b' '];
    bytes.extend_from_slice(&u32_le(99));
    assert_eq!(parse(&bytes).unwrap_err(), VoxError::BadHeader);
}

#[test]
fn parse_accepts_version_200() {
    // v200 is accepted (no spec changes our loader cares about).
    let mut bytes = vec![b'V', b'O', b'X', b' '];
    bytes.extend_from_slice(&u32_le(200));
    // No chunks — a totally empty (but valid) file. Must not return BadHeader.
    let result = parse(&bytes);
    assert!(
        !matches!(result, Err(VoxError::BadHeader)),
        "v200 must be accepted, got {result:?}"
    );
}

#[test]
fn parse_rejects_truncated_chunk() {
    // SIZE chunk declares 12 bytes of payload but file ends early.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"VOX ");
    bytes.extend_from_slice(&u32_le(150));
    bytes.extend_from_slice(b"SIZE");
    bytes.extend_from_slice(&u32_le(12)); // declares 12 bytes
    bytes.extend_from_slice(&u32_le(0)); // children_size
    bytes.extend_from_slice(&u32_le(1)); // only 4 of the 12 bytes present
    assert_eq!(parse(&bytes).unwrap_err(), VoxError::UnexpectedEof);
}

#[test]
fn parse_rejects_oversized_model() {
    let mut size = Vec::new();
    size.extend_from_slice(&u32_le(257)); // > MAX_MODEL_SIZE
    size.extend_from_slice(&u32_le(1));
    size.extend_from_slice(&u32_le(1));
    let bytes = vox_file(&[(b"SIZE", size)]);
    match parse(&bytes).unwrap_err() {
        VoxError::InvalidData(m) => assert!(m.contains("257"), "message: {m}"),
        other => panic!("expected InvalidData, got {other:?}"),
    }
}

#[test]
fn parse_rejects_negative_model_size() {
    let mut size = Vec::new();
    size.extend_from_slice(&u32_le(u32::MAX)); // -1 via two's complement
    size.extend_from_slice(&u32_le(1));
    size.extend_from_slice(&u32_le(1));

    let mut xyzi = Vec::new();
    xyzi.extend_from_slice(&u32_le(0)); // force model allocation path

    let bytes = vox_file(&[(b"SIZE", size), (b"XYZI", xyzi)]);
    match parse(&bytes).unwrap_err() {
        VoxError::InvalidData(m) => assert!(m.contains("-1"), "message: {m}"),
        other => panic!("expected InvalidData, got {other:?}"),
    }
}

#[test]
fn parse_rejects_duplicate_node_id() {
    let mut ngrp = Vec::new();
    ngrp.extend_from_slice(&u32_le(5)); // node id 5
    ngrp.extend_from_slice(&dict_payload(&[]));
    ngrp.extend_from_slice(&u32_le(0)); // 0 children
    let ngrp2 = ngrp.clone(); // same id 5
    let bytes = vox_file(&[(b"nGRP", ngrp), (b"nGRP", ngrp2)]);
    match parse(&bytes).unwrap_err() {
        VoxError::InvalidData(m) => assert!(m.contains("already exists"), "message: {m}"),
        other => panic!("expected InvalidData, got {other:?}"),
    }
}

#[test]
fn parse_rejects_dangling_child_reference() {
    // nGRP references child id 99, which is never defined.
    let mut ngrp = Vec::new();
    ngrp.extend_from_slice(&u32_le(0)); // node id 0
    ngrp.extend_from_slice(&dict_payload(&[]));
    ngrp.extend_from_slice(&u32_le(1)); // 1 child
    ngrp.extend_from_slice(&u32_le(99)); // missing child id
    let bytes = vox_file(&[(b"nGRP", ngrp)]);
    match parse(&bytes).unwrap_err() {
        VoxError::BadSceneGraph(m) => assert!(m.contains("does not exist"), "message: {m}"),
        other => panic!("expected BadSceneGraph, got {other:?}"),
    }
}

// ===========================================================================
// Data accessors
// ===========================================================================

#[test]
fn data_accessor_roundtrip() {
    // `material_id_for_palette_index` returns -1 when absent.
    let data = Data::default();
    assert_eq!(data.material_id_for_palette_index(42), -1);
    assert_eq!(data.root_node_id(), -1);
    assert_eq!(data.model_count(), 0);
    assert_eq!(data.layer_count(), 0);
}

#[test]
fn node_common_helper_returns_id_regardless_of_variant() {
    use super::data::{GroupNode, NodeCommon, ShapeNode, TransformNode};
    let t = Node::Transform(TransformNode {
        common: NodeCommon {
            id: 7,
            ..Default::default()
        },
        ..Default::default()
    });
    let g = Node::Group(GroupNode {
        common: NodeCommon {
            id: 8,
            ..Default::default()
        },
        ..Default::default()
    });
    let s = Node::Shape(ShapeNode {
        common: NodeCommon {
            id: 9,
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(t.id(), 7);
    assert_eq!(g.id(), 8);
    assert_eq!(s.id(), 9);
}
