# `voxel-gdext` — Godot 4 GDExtension bindings

The thin binding layer: the only crate in the workspace that depends on the
[`godot`](https://godot-rust.github.io) crate (gdext) and exposes Rust symbols
to GDScript via `#[func]`/`#[base]`/`#[signal]`. All engine-agnostic logic lives
in [`voxel-core`](../voxel-core); this crate wraps it into Godot classes.

## Status

Phase 2 skeleton. A single `VoxelRustHello` class with two `#[func]` methods
proves the end-to-end path (voxel-core → voxel-gdext → Godot 4.7 → GDScript)
works. Real voxel classes (`VoxelBuffer`, `VoxelTerrain`, …) land in later
phases as `voxel-core` grows the compute layer.

## Build

```sh
cd rust
cargo build -p voxel-gdext              # debug .so/.dylib/.dll
cargo build -p voxel-gdext --release    # optimized
```

This is a `cdylib`; the artifact is `target/<profile>/libvoxel_gdext.so` (Linux),
`libvoxel_gdext.dylib` (macOS), or `voxel_gdext.dll` (Windows).

## Load in Godot 4.7

1. Copy `voxel_gdext.gdextension.in` → `voxel_gdext.gdextension` somewhere under
   `res://` (e.g. the crate dir), and adjust the library paths to match where
   your built artifact lives.
2. (Re)open the Godot project — the editor scans for `.gdextension` files and
   loads the library on startup.
3. The class is now available in GDScript:

```gdscript
var v = VoxelRustHello.new()
print(v.say_hello("World"))   # "Hello, World! voxel-core v0.1.0 says hi from Rust"
print(v.is_alive())           # true
v.free()
```

### Verified

Tested headless against Godot 4.7.stable on Linux x86_64:

```
Initialize godot-rust (API v4.7.stable.official, runtime v4.7.stable.arch_linux, safeguards strict)
voxel-gdext: Scene stage initialized (voxel-core v0.1.0)
class: VoxelRustHello
hello: Hello, World! voxel-core v0.1.0 says hi from Rust
alive: true
```

The crate-to-crate path works: `say_hello()` reads `voxel_core::VERSION`,
proving `voxel-gdext` links `voxel-core` and reaches it through the FFI boundary.

## Android

The `.gdextension.in` includes an `android.arm64` entry, but producing the
`.so` needs the Android NDK (not installed in the dev environment). Use
`rust/scripts/android-build.sh --so` once the NDK is present — it already
works around the rustc↔NDK LLVM skew (see `REPORT.md`).
