//! Empty library root for the `tsan` workspace member.
//!
//! The actual ThreadSanitizer targets live in `tests/` as integration tests so
//! they exercise `voxel-core` purely through its public API. Run them with:
//!
//! ```text
//! RUSTFLAGS="-Zsanitizer=thread -Cunsafe-allow-abi-mismatch=sanitizer" \
//!   cargo +nightly test -p tsan --test <name> -- --test-threads=1
//! ```
