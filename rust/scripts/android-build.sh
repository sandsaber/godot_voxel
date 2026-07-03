#!/usr/bin/env bash
# Cross-build voxel-core for Android, working around the rustc↔NDK LLVM skew.
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
# Usage:
#   ./android-build.sh                # voxel-core release staticlib (.a), aarch64
#   ./android-build.sh --so           # shared library (.so) — exercises the NDK linker
#   ./android-build.sh --target x86_64-linux-android --so
#   ./android-build.sh --debug
#   ANDROID_API=24 ./android-build.sh --so

set -euo pipefail

TARGET=aarch64-linux-android
ANDROID_API="${ANDROID_API:-21}"
PROFILE=release
OUT_TYPE=staticlib   # staticlib (.a) | cdylib (.so)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --so)        OUT_TYPE=cdylib; shift;;
        --a)         OUT_TYPE=staticlib; shift;;
        --target)    TARGET="$2"; shift 2;;
        --debug)     PROFILE=dev; shift;;
        --release)   PROFILE=release; shift;;
        --)          shift; break;;
        *) echo "unknown arg: $1" >&2; exit 2;;
    esac
done

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
export CARGO_TARGET_$(echo "$TARGET" | tr 'a-z-' 'A-Z_')_LINKER="$CLANG"
export CARGO_TARGET_$(echo "$TARGET" | tr 'a-z-' 'A-Z_')_AR="$NDK_BIN/llvm-ar"
# Force the NDK clang to link with rust's lld instead of its own (older) lld.
export RUSTFLAGS="-C link-arg=-fuse-ld=lld -C link-arg=-B$LLD_DIR"

case "$PROFILE" in
    release) CARG_PROFILE_FLAG=--release;;
    dev)     CARG_PROFILE_FLAG=;;
esac

case "$OUT_TYPE" in
    staticlib)
        echo "→ $TARGET release staticlib (.a; NDK not strictly required)"
        cargo rustc $CARG_PROFILE_FLAG --target "$TARGET" -p voxel-core -- --crate-type staticlib
        ;;
    cdylib)
        echo "→ $TARGET $PROFILE shared library (.so; uses NDK sysroot + rust-lld)"
        cargo rustc $CARG_PROFILE_FLAG --target "$TARGET" -p voxel-core -- --crate-type cdylib
        SO="$(find "target/$TARGET/$PROFILE/deps" -name 'libvoxel_core-*.so' | head -1)"
        [[ -n "$SO" ]] && echo "produced: $SO" && file "$SO"
        ;;
esac
