// C++ reference harness for the transvoxel regular mesher.
//
// Produces a GoldenMesh JSON for an SDF sphere — identical input to the Rust
// port's `tests/transvoxel_sphere.rs` — by invoking the REAL upstream
// `build_regular_mesh<float, NullProcessor>` template. The output uses the same
// schema (`generator: "godot_voxel-cpp"`) so the Rust `matches_golden_*` parity
// tests become true H1 byte-equivalence checks.
//
// Also times the mesher run for the H2 perf baseline (printed to stderr).
//
// Built by `build_mesh.sh`, which assembles a stub header tree so this TU can
// compile transvoxel.cpp against the engine-independent parts only (no
// godot-cpp / no Godot source). See cpp-baseline/README.md.
//
// JSON is written to stdout; timing/log to stderr.

#include "meshers/transvoxel/transvoxel.h"   // pulls in transvoxel.cpp via its #include of tables.cpp? no — see below
#include "meshers/transvoxel/transvoxel.cpp" // the real algorithm (template body + dispatcher)

#include <cstdio>
#include <chrono>
#include <cmath>
#include <string>
#include <vector>

namespace tvox = zylann::voxel::transvoxel;

// ---------------------------------------------------------------------------
// SDF sphere — identical to Rust `SphereInput::new`.
// ---------------------------------------------------------------------------

static std::vector<float> make_sphere_sdf(int inner, float radius, /*out*/ Vector3i &block_size) {
    const int MIN_PADDING = 1;
    const int MAX_PADDING = 2;
    const int sz = inner + MIN_PADDING + MAX_PADDING;
    block_size = Vector3i(sz, sz, sz);
    std::vector<float> data(static_cast<size_t>(sz) * sz * sz, 0.0f);
    const float cx = inner * 0.5f;
    const float cy = inner * 0.5f;
    const float cz = inner * 0.5f;
    const int sy = sz;
    const int sx = sz;
    for (int z = 0; z < sz; ++z) {
        for (int y = 0; y < sz; ++y) {
            for (int x = 0; x < sz; ++x) {
                const float ix = static_cast<float>(x - MIN_PADDING);
                const float iy = static_cast<float>(y - MIN_PADDING);
                const float iz = static_cast<float>(z - MIN_PADDING);
                const float d = std::sqrt((ix - cx) * (ix - cx) + (iy - cy) * (iy - cy) + (iz - cz) * (iz - cz)) - radius;
                // POSITIVE = inside solid (engine SDF convention); the mesher negates.
                const float stored = -d;
                const int i = y + sy * (x + sx * z);
                data[static_cast<size_t>(i)] = stored;
            }
        }
    }
    return data;
}

// ---------------------------------------------------------------------------
// JSON helpers (hand-rolled to avoid a JSON dependency).
// ---------------------------------------------------------------------------

static void emit_f32_array(std::string &out, const char *indent, const std::vector<float> &v) {
    out += "[\n";
    for (size_t i = 0; i < v.size(); ++i) {
        if (i) out += ", ";
        if (i % 8 == 0) out += indent;
        char buf[48];
        std::snprintf(buf, sizeof(buf), "%.8g", v[i]);
        out += buf;
    }
    out += "\n  ]";
}

static void emit_i32_array(std::string &out, const char *indent, const std::vector<int32_t> &v) {
    out += "[\n";
    for (size_t i = 0; i < v.size(); ++i) {
        if (i) out += ", ";
        if (i % 12 == 0) out += indent;
        out += std::to_string(v[i]);
    }
    out += "\n  ]";
}

static void emit_u8_array(std::string &out, const char *indent, const std::vector<uint8_t> &v) {
    out += "[\n";
    for (size_t i = 0; i < v.size(); ++i) {
        if (i) out += ", ";
        if (i % 16 == 0) out += indent;
        out += std::to_string(static_cast<int>(v[i]));
    }
    out += "\n  ]";
}

int main(int argc, char **argv) {
    int inner = 16;
    float radius = 6.0f;
    const char *out_name = "transvoxel_sphere_16.json";
    if (argc >= 2) inner = std::atoi(argv[1]);
    if (argc >= 3) radius = static_cast<float>(std::atof(argv[2]));
    if (argc >= 4) out_name = argv[3];
    if (argc >= 5) {
        std::fprintf(stderr, "usage: %s [inner] [radius] [out_name]\n", argv[0]);
        return 2;
    }

    Vector3i block_size;
    std::vector<float> sdf = make_sphere_sdf(inner, radius, block_size);

    // Wrap the flat data in the mesher's Span.
    zylann::Span<const float> sdf_span(sdf.data(), sdf.size());

    tvox::Cache cache;
    tvox::MeshArrays output;
    std::vector<tvox::CellInfo> cell_infos;
    const uint32_t lod_index = 0;
    const float edge_clamp_margin = 0.0f;

    // Time the mesher run (H2 baseline). Several iterations for stability.
    const int iters = 50;
    auto t0 = std::chrono::steady_clock::now();
    for (int i = 0; i < iters; ++i) {
        output.clear();
        cache.reset_reuse_cells(block_size);
        tvox::build_regular_mesh<float, tvox::materials::NullProcessor>(
            sdf_span,
            tvox::materials::NullProcessor{},
            block_size,
            lod_index,
            cache,
            output,
            /*cell_info*/ nullptr,
            edge_clamp_margin
        );
    }
    auto t1 = std::chrono::steady_clock::now();
    double ns_total = static_cast<double>(std::chrono::duration_cast<std::chrono::nanoseconds>(t1 - t0).count());
    double ms_per_run = ns_total / double(iters) / 1e6;
    // Mvoxels/s = (inner^3) / (seconds per run) / 1e6. inner^3 is the "useful" voxel count.
    double voxels = static_cast<double>(inner) * inner * inner;
    double seconds = ns_total / double(iters) / 1e9;
    double mvoxels_per_s = voxels / seconds / 1e6;
    std::fprintf(stderr, "mesh: %d inner, r=%.1f -> %zu verts, %zu indices\n",
        inner, radius, output.vertices.size(), output.indices.size());
    std::fprintf(stderr, "timing: %d iters, %.4f ms/run, %.2f Mvoxels/s\n",
        iters, ms_per_run, mvoxels_per_s);

    // Emit GoldenMesh JSON (schema must match Rust's GoldenMesh exactly).
    std::vector<float> vertices_f, normals_f, secondary_f;
    std::vector<int32_t> indices = output.indices;
    std::vector<uint8_t> cell_border_masks, vertex_border_masks, transitions;
    vertices_f.reserve(output.vertices.size() * 3);
    normals_f.reserve(output.normals.size() * 3);
    secondary_f.reserve(output.lod_data.size() * 3);
    cell_border_masks.reserve(output.lod_data.size());
    vertex_border_masks.reserve(output.lod_data.size());
    transitions.reserve(output.lod_data.size());
    for (const auto &v : output.vertices) { vertices_f.push_back(v.x); vertices_f.push_back(v.y); vertices_f.push_back(v.z); }
    for (const auto &n : output.normals) { normals_f.push_back(n.x); normals_f.push_back(n.y); normals_f.push_back(n.z); }
    for (const auto &ld : output.lod_data) {
        secondary_f.push_back(ld.secondary_position.x);
        secondary_f.push_back(ld.secondary_position.y);
        secondary_f.push_back(ld.secondary_position.z);
        cell_border_masks.push_back(ld.cell_border_mask);
        vertex_border_masks.push_back(ld.vertex_border_mask);
        transitions.push_back(ld.transition);
    }

    std::string json;
    json += "{\n";
    json += "  \"schema_version\": 1,\n";
    json += "  \"generator\": \"godot_voxel-cpp\",\n";
    json += "  \"algorithm\": \"transvoxel/regular\",\n";
    json += "  \"input\": {\n";
    json += "    \"kind\": \"sphere\",\n";
    json += "    \"inner\": " + std::to_string(inner) + ",\n";
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%.8g", radius);
    json += "    \"radius\": "; json += buf; json += ",\n";
    json += "    \"min_padding\": 1,\n";
    json += "    \"max_padding\": 2\n";
    json += "  },\n";
    json += "  \"params\": {\n";
    json += "    \"lod_index\": 0,\n";
    json += "    \"edge_clamp_margin\": 0.0\n";
    json += "  },\n";
    json += "  \"vertex_count\": " + std::to_string(output.vertices.size()) + ",\n";
    json += "  \"index_count\": " + std::to_string(output.indices.size()) + ",\n";
    json += "  \"vertices\": ";     emit_f32_array(json, "    ", vertices_f);          json += ",\n";
    json += "  \"normals\": ";      emit_f32_array(json, "    ", normals_f);           json += ",\n";
    json += "  \"secondary_positions\": "; emit_f32_array(json, "    ", secondary_f);  json += ",\n";
    json += "  \"indices\": ";      emit_i32_array(json, "    ", indices);             json += ",\n";
    json += "  \"cell_border_masks\": ";   emit_u8_array(json, "    ", cell_border_masks);   json += ",\n";
    json += "  \"vertex_border_masks\": "; emit_u8_array(json, "    ", vertex_border_masks); json += ",\n";
    json += "  \"transitions\": ";         emit_u8_array(json, "    ", transitions);         json += "\n";
    json += "}\n";

    if (argc >= 4) {
        // Write to the named file (used when regenerating goldens).
        std::FILE *f = std::fopen(out_name, "wb");
        if (!f) { std::fprintf(stderr, "cannot open %s\n", out_name); return 1; }
        std::fwrite(json.data(), 1, json.size(), f);
        std::fclose(f);
        std::fprintf(stderr, "wrote %s\n", out_name);
    } else {
        std::fwrite(json.data(), 1, json.size(), stdout);
    }
    return 0;
}
