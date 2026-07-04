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

`voxel-gdext` cross-compiles for Android arm64 (device) and x86_64 (emulator)
with the NDK. The build script works around the rustc↔NDK LLVM skew (NDK r29's
`lld` can't parse rustc/LLVM-22 objects; the script forces rust's `lld` via
`-fuse-ld=lld`) and exports `CC`/`CXX` so the `godot` crate builds `godot-cpp`
for the same target.

```sh
cd rust
./scripts/android-build.sh                                  # aarch64 .so (device)
./scripts/android-build.sh --target x86_64-linux-android    # x86_64 .so (emulator)
./scripts/android-build.sh --strip                          # strip debug symbols
```

Verified with NDK r29 (14206865), `ANDROID_API=21`, Godot `api-4-7`: produces
`target/<triple>/release/libvoxel_gdext.so` exporting `gdext_rust_init`
(~3.2 MB unstripped). The `.gdextension.in` carries matching `android.arm64`
and `android.x86_64` entries.

Loading the `.so` inside a Godot Android export template still requires a
custom template compiled with `platform=android` (the stock template does not
load GDExtensions at runtime on device) — that packaging step is tracked as
the remaining Phase 2 mobile-half item (needs an SDK + device/emulator).
