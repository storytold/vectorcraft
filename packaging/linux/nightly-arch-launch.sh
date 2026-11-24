#!/bin/sh
set -eu

real_bin="$(dirname "$0")/.vectorcraft-nightly/vectorcraft"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$real_bin" "$@"
fi
profile_base="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
exec "$real_bin" --launch-profile nightly-v1 --profile-root "$profile_base/vectorcraft-nightly" -- "$@"
