//! libFuzzer target: region-file block payloads (audit §9.6 «Инфраструктура»).
//!
//! Region files store voxel blocks in an on-disk sector layout where each
//! block payload is a `block_serializer` envelope. This target fuzzes those
//! payloads directly — the same deserialization path `RegionFile::load_block`
//! uses after it has located and read the sector bytes. Full region-header /
//! LUT / sector-index fuzzing would require exposing `RegionFile::load_header`
//! publicly (it is currently `pub(crate)`); tracked as a follow-up.
//!
//! Run: `cargo +nightly fuzz run region_file`

#![no_main]

use libfuzzer_sys::fuzz_target;
use voxel_core::storage::VoxelBuffer;

fuzz_target!(|data: &[u8]| {
    // Each region sector contains a block_serializer payload; deserializing it
    // from arbitrary bytes exercises the same LZ4 + block-format parser the
    // region loader uses internally.
    let mut buffer = VoxelBuffer::with_size(voxel_core::math::Vector3i::zero());
    let _ = voxel_core::streams::block_serializer::decompress_and_deserialize(data, &mut buffer);
});
