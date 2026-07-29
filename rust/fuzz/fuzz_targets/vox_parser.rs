//! libFuzzer target: MagicaVoxel `.vox` parser (audit §9.6 «Инфраструктура»).
//!
//! Feeds the fuzzer-supplied byte slice into [`voxel_core::format::vox::parse`].
//! The parser must not panic on arbitrary input; malformed files are expected
//! to return `Err(VoxError)`.
//!
//! Run: `cargo +nightly fuzz run vox_parser`

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = voxel_core::format::vox::parse(data);
});
