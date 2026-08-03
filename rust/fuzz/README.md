# `voxel-core-fuzz` — cargo-fuzz / libFuzzer targets

Byte-level parser fuzzing for `voxel-core`. Three targets, one per untrusted
input format:

| Target | Entry point |
|---|---|
| `vox_parser` | `format::vox::parse` (MagicaVoxel `.vox` files) |
| `block_serializer` | `streams::block_serializer::decompress_and_deserialize` |
| `region_file` | same envelope parser, as used by region-file block payloads |

## Running

Requires nightly and `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz

cd rust
cargo +nightly fuzz run vox_parser            # uses fuzz/corpus/vox_parser
```

## Seed corpora

The working corpus (`corpus/`) and crash artifacts (`artifacts/`) are
git-ignored and machine-local. Committed, validated starter inputs live in
`seed_corpus/<target>/`. Use them as the starting corpus:

```sh
cargo +nightly fuzz run vox_parser fuzz/seed_corpus/vox_parser
cargo +nightly fuzz run block_serializer fuzz/seed_corpus/block_serializer
cargo +nightly fuzz run region_file fuzz/seed_corpus/region_file
```

Regenerate the seeds (they are produced by real voxel-core code paths, and the
generator asserts every seed round-trips through the fuzzed entry point):

```sh
cargo run -p voxel-core --example gen_fuzz_seeds
```

## Known gap

Full region-file header/LUT fuzzing is blocked on `RegionFile::load_header`
being `pub(crate)`; the `region_file` target covers block payloads only.
