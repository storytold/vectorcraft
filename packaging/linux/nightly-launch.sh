#!/bin/sh
# The launcher inside VectorCraft Nightly's flat tar: an unpacked copy is isolated like the packages.
set -eu

here="$(dirname "$0")"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$here/vectorcraft" "$@"
fi
profile_base="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
exec "$here/vectorcraft" --launch-profile nightly-v1 --profile-root "$profile_base/vectorcraft-nightly" -- "$@"
