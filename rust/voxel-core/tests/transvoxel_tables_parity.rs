//! Table-level parity: Rust transvoxel tables vs the REAL upstream C++ tables.
//!
//! `golden/transvoxel_tables_cpp.txt` is produced by `rust/cpp-baseline/` from
//! upstream's `meshers/transvoxel/transvoxel_tables.cpp` (compiled standalone
//! with a stub for its only include). This test asserts the Rust port's three
//! regular-cell tables are byte-for-byte identical to that dump.
//!
//! Table parity is necessary but not sufficient for full mesh parity: it proves
//! the lookup backbone matches, so any future mesh-parity difference would have
//! to come from the vertex-interpolation / reuse-cache logic (a small, mirrored
//! part of the algorithm). Full mesh parity (H1) needs the godot-cpp harness.

// Index loops are intentional: we compare element-by-element with a precise
// position in each failure message so any divergence points straight at the cell.
#![allow(clippy::needless_range_loop)]

use voxel_core::meshers::transvoxel::regular_tables::{
    get_regular_cell_class, get_regular_cell_data, get_regular_vertex_data, REGULAR_CELL_CLASS,
    REGULAR_CELL_DATA, REGULAR_VERTEX_DATA,
};

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

/// Parse the C++ dump and compare every value to the Rust tables.
#[test]
fn rust_tables_match_cpp_dump() {
    let text = std::fs::read_to_string(golden_path("transvoxel_tables_cpp.txt"))
        .expect("C++ table dump missing — run rust/cpp-baseline/build.sh");
    let mut it = text.split_whitespace();

    // Header.
    assert_eq!(it.next(), Some("TRANSVOXEL_TABLES_CPP"), "bad dump header");
    let version: u32 = it.next().unwrap().parse().expect("schema version");
    assert_eq!(version, 1, "bump parser if the dump schema changes");

    // REGULAR_CELL_CLASS[256].
    assert_eq!(it.next(), Some("REGULAR_CELL_CLASS"));
    let n: usize = it.next().unwrap().parse().unwrap();
    assert_eq!(n, 256);
    for i in 0..256 {
        let cpp: u8 = it.next().unwrap().parse().unwrap();
        assert_eq!(
            cpp, REGULAR_CELL_CLASS[i],
            "regularCellClass[{i}] mismatch (cpp={cpp})"
        );
        assert_eq!(cpp, get_regular_cell_class(i as u8));
    }

    // REGULAR_CELL_DATA[16]: geometryCounts + 15 vertexIndex each.
    assert_eq!(it.next(), Some("REGULAR_CELL_DATA"));
    let n: usize = it.next().unwrap().parse().unwrap();
    assert_eq!(n, 16);
    for i in 0..16 {
        let cd = REGULAR_CELL_DATA[i];
        let cpp_geom: u8 = it.next().unwrap().parse().unwrap();
        assert_eq!(
            cpp_geom, cd.geometry_counts,
            "regularCellData[{i}].geometryCounts mismatch"
        );
        for j in 0..15 {
            let cpp_v: u8 = it.next().unwrap().parse().unwrap();
            assert_eq!(
                cpp_v, cd.vertex_index[j],
                "regularCellData[{i}].vertexIndex[{j}] mismatch"
            );
            assert_eq!(cpp_v, get_regular_cell_data(i as u8).get_vertex_index(j));
        }
    }

    // REGULAR_VERTEX_DATA[256][12].
    assert_eq!(it.next(), Some("REGULAR_VERTEX_DATA"));
    let rows: usize = it.next().unwrap().parse().unwrap();
    let cols: usize = it.next().unwrap().parse().unwrap();
    assert_eq!((rows, cols), (256, 12));
    for i in 0..256 {
        for j in 0..12 {
            let cpp_v: u16 = it.next().unwrap().parse().unwrap();
            assert_eq!(
                cpp_v, REGULAR_VERTEX_DATA[i][j],
                "regularVertexData[{i}][{j}] mismatch (cpp={cpp_v})"
            );
            assert_eq!(cpp_v, get_regular_vertex_data(i as u8, j as u8));
        }
    }

    // Nothing trailing.
    assert_eq!(it.next(), None, "unexpected trailing tokens in C++ dump");
}
