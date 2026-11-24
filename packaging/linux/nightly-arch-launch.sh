#!/bin/sh
set -eu

real_bin="$(dirname "$0")/.vectorcraft-nightly/vectorcraft"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$real_bin" "$@"
fi
if [ -z "${VECTORCRAFT_CONFIG_DIR+x}" ]; then
  profile_base="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
  export VECTORCRAFT_CONFIG_DIR="$profile_base/vectorcraft-nightly"
fi
exec "$real_bin" "$@"
