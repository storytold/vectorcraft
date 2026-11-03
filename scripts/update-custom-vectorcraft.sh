#!/bin/sh
# Update the custom VectorCraft build without dropping local tool-plugin support.
set -eu
cd "$(dirname "$0")/.."

branch=feature/tangent-circle-snap
current=$(git branch --show-current)
if [ "$current" != "$branch" ]; then
    echo "switch to $branch first (currently on ${current:-detached HEAD})" >&2
    exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
    echo "the worktree is not clean; commit or stash changes before updating" >&2
    exit 1
fi

git fetch origin
git rebase origin/main
cargo test -p vectorcraft-plugins --test abi
cargo test -p vectorcraft-tools plugin --lib
cargo test -p vectorcraft-engine tangent_circle_plugin_keeps_an_exact_external_tangency_through_undo_redo --lib
cargo test --manifest-path plugins/tangent-circle/Cargo.toml
plugins/tangent-circle/build.sh
cargo xtask bundle

echo
echo "Updated custom build: $(pwd)/dist/VectorCraft.app"
echo "Installable plug-in: $(cd ../.. && pwd)/outputs/TangentCircle.wasm"
echo "The branch kept its host extension. Do not accept the official in-app updater over this custom app."
