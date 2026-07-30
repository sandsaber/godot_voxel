#!/usr/bin/env bash
# Builds the voxel-gdext library and runs the headless Godot smoke tests.
#
# The compiled `.so`/`.dylib`/`.dll` is a build artifact (git-ignored), so on a
# clean checkout it must be produced before Godot can load the GDExtension.
# This script does that, copies the artifact next to the .gdextension, then runs
# both checks. Requires `cargo` and `godot` on PATH.
#
# Usage:  ./voxel-gdext/smoke_test/run_smoke_test.sh [--release]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_RUST="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROFILE="debug"
GODOT="${GODOT:-godot}"

while [[ $# -gt 0 ]]; do
	case "$1" in
		--release) PROFILE="release"; shift;;
		*) echo "unknown arg: $1" >&2; exit 2;;
	esac
done

cd "$REPO_RUST"
echo ">> building voxel-gdext ($PROFILE)..."
cargo build -p voxel-gdext ${PROFILE:+$([ "$PROFILE" = release ] && echo --release)}

# Copy the artifact next to the .gdextension (which points at res://libvoxel_gdext.so).
EXT="so"; [[ "$(uname -s)" == "Darwin" ]] && EXT="dylib"
SRC="$REPO_RUST/target/$PROFILE/libvoxel_gdext.$EXT"
DST="$SCRIPT_DIR/libvoxel_gdext.$EXT"
cp -f "$SRC" "$DST"
echo ">> copied $SRC -> $DST"

echo
echo ">> [1/3] API test (class registration + func surface)..."
"$GODOT" --headless --path "$SCRIPT_DIR" --script api_test.gd

echo
echo ">> [2/3] runtime paging test (terrain + generator + viewer, real frames)..."
"$GODOT" --headless --path "$SCRIPT_DIR" runtime_scene.tscn --quit-after 120

echo
echo ">> [3/3] smoke scene (VoxelTerrain node in a scene)..."
"$GODOT" --headless --path "$SCRIPT_DIR" smoke_test.tscn --quit-after 30

echo
echo ">> all smoke tests complete"
