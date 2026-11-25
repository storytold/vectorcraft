#!/usr/bin/env bash
# Stage the VectorCraft Nightly-only handoff for the Arch packager.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
source_tree="$1"
work="$2"
destination="$3"
basename="$4"
stage="$work/$basename"

mkdir -p "$work"
cp -R "$source_tree" "$stage"
mkdir "$stage/bin/.vectorcraft-nightly"
# The GUI goes into the hidden folder; `bin/vectorcraft` below is the launcher that starts it.
mv "$stage/bin/vectorcraft-bin" "$stage/bin/.vectorcraft-nightly/vectorcraft-bin"
cp "$here/nightly-launch.sh" "$stage/bin/vectorcraft"
chmod 755 "$stage/bin/vectorcraft"
tar -C "$work" -czf "$destination/$basename.tar.gz" "$basename"
echo "wrote $destination/$basename.tar.gz"
