#!/usr/bin/env bash
# Cross-build the Rust voxel workspace for Android, working around the
# rustc↔NDK LLVM skew.
#
# Context (Phase 0 finding, see REPORT.md): rustc 1.96.1 ships LLVM 22; NDK r29
# ships LLVM 21. The NDK's bundled lld cannot read objects emitted by the newer
# rustc ("Unknown attribute kind (103)"), so linking a final .so fails. The .a
# (static archive) is unaffected because archiving doesn't parse object attrs.
#
# Fix: keep the NDK clang as the linker DRIVER (it provides the Android sysroot
# and libc), but force it to link with rust's bundled lld (LLVM 22) via
# `-fuse-ld=lld` plus a search-path symlink named `ld.lld` → rust-lld.
#
# The same clang is also exported as CC/CXX so the `godot` crate's build of
# `godot-cpp` (C++) cross-compiles for the Android target under voxel-gdext.
#
# Usage:
#   ./android-build.sh                      # voxel-gdext release .so, aarch64 (device)
#   ./android-build.sh --target x86_64-linux-android   # .so for emulator
#   ./android-build.sh --core               # voxel-core staticlib (.a) only
#   ./android-build.sh --core --so          # voxel-core cdylib (.so)
#   ./android-build.sh --debug
#   ./android-build.sh --strip              # strip the produced .so
#   ANDROID_API=24 ./android-build.sh

set -euo pipefail

TARGET=aarch64-linux-android
ANDROID_API="${ANDROID_API:-21}"
PROFILE=release
CRATE=gdext          # gdext (voxel-gdext .so) | core (voxel-core .a or .so)
# Default per-crate output: gdext is always cdylib; core defaults to staticlib
# (matching the original Phase 0 behaviour), unless --so flips it to cdylib.
OUT_TYPE=staticlib   # staticlib (.a) | cdylib (.so); overridden for gdext below
STRIP=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --so)        OUT_TYPE=cdylib; shift;;
        --a)         OUT_TYPE=staticlib; CRATE=core; shift;;
        --core)      CRATE=core; shift;;
        --gdext)     CRATE=gdext; shift;;
        --target)    TARGET="$2"; shift 2;;
        --debug)     PROFILE=dev; shift;;
        --release)   PROFILE=release; shift;;
        --strip)     STRIP=1; shift;;
        --)          shift; break;;
        *) echo "unknown arg: $1" >&2; exit 2;;
    esac
done

# `--so` on its own implies gdext (a .so is the GDExtension artifact). `--core`
# keeps whatever OUT_TYPE was set (default staticlib, or cdylib via `--core --so`).
if [[ "$CRATE" == "gdext" && "$OUT_TYPE" == "staticlib" ]]; then
    OUT_TYPE=cdylib   # gdext has no staticlib form; it's a cdylib by definition
fi

# --- locate the NDK ----------------------------------------------------------
NDK_ROOT="${ANDROID_NDK_HOME:-${ANDROID_NDK:-}}"
if [[ -z "$NDK_ROOT" ]]; then
    for cand in /opt/android-ndk "$HOME/Android/Sdk/ndk"/*; do
        [[ -d "$cand/toolchains/llvm" ]] && NDK_ROOT="$cand" && break
    done
fi
if [[ -z "$NDK_ROOT" || ! -d "$NDK_ROOT/toolchains/llvm" ]]; then
    echo "Android NDK not found. Set ANDROID_NDK_HOME or install to /opt/android-ndk." >&2
    exit 1
fi
NDK_BIN="$NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin"
ARCH="${TARGET%%-*}"   # aarch64 | x86_64
CLANG="$NDK_BIN/$ARCH-linux-android${ANDROID_API}-clang"
CLANGXX="$NDK_BIN/$ARCH-linux-android${ANDROID_API}-clang++"
echo "NDK:        $NDK_ROOT"
echo "clang:      $CLANG"

# --- locate rust's lld (same LLVM as rustc) ---------------------------------
RUST_LLD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld"
if [[ ! -x "$RUST_LLD" ]]; then
    echo "rust-lld not found at $RUST_LLD" >&2
    exit 1
fi
LLD_DIR="$(mktemp -d)"
trap 'rm -rf "$LLD_DIR"' EXIT
# clang -fuse-ld=lld looks up an executable named `ld.lld`; rust-lld dispatches
# on argv[0], so a symlink named ld.lld makes it behave as the GNU ELF linker.
ln -sf "$RUST_LLD" "$LLD_DIR/ld.lld"
echo "rust-lld:   $RUST_LLD"

# --- drive cargo ------------------------------------------------------------
cd "$(dirname "$0")/.."   # rust/ workspace root
TARGET_ENV="$(echo "$TARGET" | tr 'a-z-' 'A-Z_')"
export CARGO_TARGET_${TARGET_ENV}_LINKER="$CLANG"
export CARGO_TARGET_${TARGET_ENV}_AR="$NDK_BIN/llvm-ar"
# Force the NDK clang to link with rust's lld instead of its own (older) lld.
export RUSTFLAGS="-C link-arg=-fuse-ld=lld -C link-arg=-B$LLD_DIR"
# godot-cpp is C++; point its `cc`/`cxx` crate builds at the NDK clang so the
# binding layer cross-compiles for the same Android target.
export CC_$TARGET_ENV="$CLANG"
export CXX_$TARGET_ENV="$CLANGXX"

case "$PROFILE" in
    release) CARGO_PROFILE_FLAG=--release;;
    dev)     CARGO_PROFILE_FLAG=;;
esac

if [[ "$CRATE" == "core" ]]; then
    case "$OUT_TYPE" in
        staticlib)
            echo "→ $TARGET $PROFILE voxel-core staticlib (.a)"
            cargo rustc $CARGO_PROFILE_FLAG --target "$TARGET" -p voxel-core -- --crate-type staticlib
            ;;
        cdylib)
            echo "→ $TARGET $PROFILE voxel-core shared library (.so)"
            cargo rustc $CARGO_PROFILE_FLAG --target "$TARGET" -p voxel-core -- --crate-type cdylib
            ;;
    esac
else   # gdext (always cdylib)
    echo "→ $TARGET $PROFILE voxel-gdext shared library (.so; uses NDK sysroot + rust-lld)"
    cargo build $CARGO_PROFILE_FLAG --target "$TARGET" -p voxel-gdext
fi

# --- locate + report the produced .so --------------------------------------
if [[ "$OUT_TYPE" == "cdylib" ]]; then
    if [[ "$CRATE" == "gdext" ]]; then
        SO="target/$TARGET/$PROFILE/libvoxel_gdext.so"
    else
        SO="$(find "target/$TARGET/$PROFILE/deps" -name 'libvoxel_core-*.so' | head -1)"
    fi
    if [[ -z "$SO" || ! -f "$SO" ]]; then
        echo "produced .so not found" >&2
        exit 1
    fi
    if [[ "$STRIP" == "1" ]]; then
        "$NDK_BIN/llvm-strip" --strip-debug "$SO"
        echo "stripped:   $SO"
    fi
    echo "produced:   $SO"
    ls -la "$SO"
    file "$SO"
    if "$NDK_BIN/llvm-nm" -D "$SO" 2>/dev/null | grep -q 'T gdext_rust_init'; then
        echo "symbol:     gdext_rust_init ✓"
    fi
fi
