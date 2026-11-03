#!/bin/sh
# Builds the installable Tangent Circle tool plug-in.
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
out=target/wasm32-unknown-unknown/release/vectorcraft_plugin_tangent_circle.wasm
mkdir -p ../../../../outputs
cp "$out" ../../../../outputs/TangentCircle.wasm
cp "$out" ../../crates/plugins/tests/fixtures/tangent_circle.wasm
echo "built $out ($(wc -c < "$out") bytes) -> ../../../../outputs/TangentCircle.wasm and the engine fixture"
