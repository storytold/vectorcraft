#!/bin/sh
# The launcher every VectorCraft Nightly format starts: the same file in the plain tar (`vectorcraft`),
# the deb/rpm (`/usr/bin/vectorcraft-nightly`), the AppImage (`AppRun`), the Arch handoff
# (`bin/vectorcraft`) and the Flatpak (`/app/bin/vectorcraft`). It starts the GUI beside it
# (`<app>-bin`, the standard Nightly layout) in the nightly-v1 channel's own profile.
#
# An invocation that carries the Launcher's protocol, or a bare version or help query, goes straight
# to the GUI: it answers those before it validates a profile root, and it treats everything after
# `--` as a file. Everything else gets the channel's profile root and the `--` delimiter.
set -eu

case "$0" in
  */*) here="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)" ;;
  # Started from PATH: the program that ran is the one to look beside.
  *) here="$(CDPATH='' cd -- "$(dirname -- "$(command -v -- "$0")")" && pwd)" ;;
esac

real_bin=
for candidate in \
  "$here/vectorcraft-bin" \
  "$here/usr/bin/vectorcraft-bin" \
  "$here/.vectorcraft-nightly/vectorcraft-bin" \
  "$here/../lib/vectorcraft-nightly/vectorcraft-bin" \
  "$here/../libexec/vectorcraft-bin"; do
  if [ -x "$candidate" ] && [ ! -d "$candidate" ]; then
    real_bin="$candidate"
    break
  fi
done
[ -n "$real_bin" ] || {
  echo 'VectorCraft Nightly: the app binary is missing beside this launcher' >&2
  exit 2
}

case "${1:-}" in
  # These branches need no profile root, so they answer even where HOME is unset.
  --version | -V | --help | -h | --launch-profile) exec "$real_bin" "$@" ;;
esac
profile_root="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}/vectorcraft-nightly"
if [ "${1:-}" = "--" ]; then
  # The caller supplied the file delimiter already: never send two.
  shift
  exec "$real_bin" --launch-profile nightly-v1 --profile-root "$profile_root" -- "$@"
fi
exec "$real_bin" --launch-profile nightly-v1 --profile-root "$profile_root" -- "$@"
