//! libFuzzer target: voxel block serializer (audit §9.6 «Инфраструктура»).
//!
//! Feeds the fuzzer-supplied byte slice into
//! [`voxel_core::streams::block_serializer::decompress_and_deserialize`], the
//! realistic on-disk entry point (LZ4 envelope → block format). The parser
//! must not panic on arbitrary input; malformed payloads return
//! `Err(block_serializer::Error)`.
//!
//! Run: `cargo +nightly fuzz run block_serializer`

#![no_main]

use libfuzzer_sys::fuzz_target;
use voxel_core::storage::VoxelBuffer;

fuzz_target!(|data: &[u8]| {
    // A throwaway buffer; the deserializer re-creates it to match the payload's
    // declared size/depth. Errors are expected and ignored — the goal is to
    // confirm no panic / out-of-bounds on arbitrary bytes.
    let mut buffer = VoxelBuffer::with_size(voxel_core::math::Vector3i::zero());
    let _ = voxel_core::streams::block_serializer::decompress_and_deserialize(data, &mut buffer);
});
