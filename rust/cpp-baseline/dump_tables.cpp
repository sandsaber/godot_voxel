// Standalone dumper for the transvoxel regular lookup tables.
//
// It includes the REAL upstream `meshers/transvoxel/transvoxel_tables.cpp`
// (copied into the build dir by `build.sh`) and prints the three regular-cell
// tables in a stable, machine-readable text format. The output is consumed by
// `voxel-core`'s Rust parity test (`transvoxel_tables_parity.rs`) to prove the
// Rust port's tables are byte-for-byte identical to upstream.
//
// This is the cheapest real C++-vs-Rust parity signal achievable without a
// full Godot/godot-cpp build (the mesher body itself needs Godot types; the
// tables are self-contained data). Table parity is necessary-but-not-sufficient
// for full mesh parity — see README.md.

#include <cstdio>

// Pulls in the upstream table data from `namespace zylann::voxel::transvoxel::tables`.
// The build dir is on the include path; the copied file's `../../util/errors.h`
// resolves to the empty stub created by build.sh.
#include "meshers/transvoxel/transvoxel_tables.cpp"

using namespace zylann::voxel::transvoxel::tables;

int main() {
    std::printf("TRANSVOXEL_TABLES_CPP 1\n");

    std::printf("REGULAR_CELL_CLASS 256\n");
    for (int i = 0; i < 256; ++i) {
        std::printf("%u%c", static_cast<unsigned>(regularCellClass[i]), i == 255 ? '\n' : ' ');
    }

    std::printf("REGULAR_CELL_DATA 16\n");
    for (int i = 0; i < 16; ++i) {
        const RegularCellData &d = regularCellData[i];
        std::printf("%u", static_cast<unsigned>(d.geometryCounts));
        for (int j = 0; j < 15; ++j) {
            std::printf(" %u", static_cast<unsigned>(d.vertexIndex[j]));
        }
        std::printf("\n");
    }

    std::printf("REGULAR_VERTEX_DATA 256 12\n");
    for (int i = 0; i < 256; ++i) {
        for (int j = 0; j < 12; ++j) {
            std::printf("%u%c", static_cast<unsigned>(regularVertexData[i][j]), j == 11 ? '\n' : ' ');
        }
    }
    return 0;
}
