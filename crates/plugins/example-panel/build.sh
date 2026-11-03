#!/bin/sh
# Builds the CSV Rows example panel plug-in.
# Needs the wasm32-unknown-unknown target: `rustup target add wasm32-unknown-unknown`.
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
out=target/wasm32-unknown-unknown/release/vectorcraft_plugin_csv_rows.wasm
echo "built crates/plugins/example-panel/$out ($(wc -c < "$out") bytes)"
