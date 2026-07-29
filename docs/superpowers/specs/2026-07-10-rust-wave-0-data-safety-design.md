# Rust Wave 0: Data Safety Design

**Goal:** Remove the P0 blockers reported in `rust/AUDIT.md` before the Rust port is used for production persistence, untrusted binary imports, or a public Godot binding.

**Scope:** This wave implements safe vector indexing; bounded and fallible decode paths for compression, block, region, and VOX readers; strict region-format and LUT validation; and a terrain-owned save journal with ordered per-block commits, bounded retries, and explicit shutdown/flush.

**Non-goals:** WAL/dual-header crash recovery, cross-process multi-handle region coherency, VOX framing/schema parity, edit transactions, mesh lifecycle, graph parity, and performance work remain in later audit waves. `shutdown_and_flush` guarantees that a clean, reported shutdown does not silently discard queued saves; it does not claim recovery from process kill or power loss.

## 0A — safe vector indexing

`Vector3T` and `Vector4T` public `get`/`set` will use exhaustive checked matches and panic on an invalid index, matching their `Index` and `IndexMut` implementations. The safe API will contain no reachable `unreachable_unchecked`. Tests cover every valid component and invalid read/write indices.

## 0B — bounded decode

`DecodeLimits` will be a public, explicit configuration carried through nested decoders. It has independent caps for decoded bytes, voxel count, model count, nodes, and strings; public convenience entry points use conservative documented defaults while callers that need larger trusted assets can supply stricter or wider limits intentionally.

All allocation sizes will use checked arithmetic and `try_reserve_exact`, returning the existing format-specific error type rather than panicking or allocating from an untrusted length. LZ4/Zstd validate the advertised output size before allocation; V4 blocks preflight dimensions, channel sizes, and remaining input; region payloads are bounded by their declared sector span; VOX parsing charges cumulative model and voxel budgets before constructing dense models.

## 0C — strict `.vxr` format validation

`RegionFormat` construction and header loading will become fallible. Validation rejects zero or wire-unrepresentable axes/sectors, overflowing volume/header calculations, invalid channel depths, and block entries outside the file or overlapping another entry.

Before allocation or reverse-sector-cache construction, header reading will validate the full LUT against the file length with interval checks. `RegionBlockInfo` creation and mutation will return an error for values that cannot be represented instead of masking bits in release builds. Existing valid region files retain their layout and read/write compatibility.

## 0D — save ownership and controlled shutdown

`VoxelTerrainCore` will own a save journal keyed by `(position_in_blocks, lod_index)`. Each entry assigns a monotonic generation and holds either a queued payload or one in-flight payload. At most one save for a key is dispatched at a time; a newer queued generation supersedes an older failed generation, while a failed current generation retains its voxel buffer for a bounded retry.

`SaveBlockDataTask` will return a typed save completion carrying its generation and, on failure or cancellation, the original payload. The terrain applies a completion only if it corresponds to the current in-flight generation, commits successful saves, and schedules eligible retries without dropping data. Independent block keys may save concurrently.

`shutdown_and_flush() -> Result` stops accepting new terrain work, drains all journal entries through their retry policy, flushes the stream only after confirmed commits, and returns an error with the retained failed keys if it cannot persist them. Drop remains best-effort only and is never the success path for persistence.

## Test strategy and definition of done

Each behavior follows a test-first red/green cycle. Deterministic fake streams and barriers prove: a failed save retains the exact payload; reverse completion order cannot overwrite a newer generation; retry limits surface failure without loss; and shutdown drains then flushes. Decode tests cover oversized advertised LZ4/Zstd output, oversized V4/region payloads, and cumulative VOX budgets. Region tests cover invalid headers, out-of-range and overlapping LUT entries, and rejected wire overflows.

Wave 0 is complete only when these regressions pass together with `cargo fmt --all -- --check`, `cargo test --workspace --all-features --locked`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and `cargo build --workspace --all-features --locked`. The audit's Wave 0 no-go conditions may then be reassessed from evidence, without claiming later-wave parity or crash durability.
