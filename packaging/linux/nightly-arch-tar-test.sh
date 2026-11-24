#!/usr/bin/env bash
# Validate the staged Arch launcher without building or opening VectorCraft.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/source"
dst="$work/dist"
base=vectorcraft-0.0.1-nightly.1-linux-x86_64
mkdir -p "$stage/bin" "$dst" "$work/extract"

# A local argv/env recorder stands in for the compiled binaries. It never opens a UI.
cat >"$stage/bin/vectorcraft" <<'RECORDER'
#!/bin/sh
printf 'CONFIG=<%s>\n' "${VECTORCRAFT_CONFIG_DIR:-}"
for arg in "$@"; do
  printf '<%s>\n' "$arg"
done
RECORDER
chmod 755 "$stage/bin/vectorcraft"
cp "$stage/bin/vectorcraft" "$stage/bin/vectorcraft-cli"

bash "$HERE/nightly-arch-tar.sh" "$stage" "$work/stage" "$dst" "$base"
tar -xzf "$dst/$base.tar.gz" -C "$work/extract"
bin="$work/extract/$base/bin"
# Simulate the existing Arch bin-tree copy and public launcher rename.
mkdir -p "$work/installed"
cp -R "$bin" "$work/installed/bin"
bin="$work/installed/bin"
mv "$bin/vectorcraft" "$bin/vectorcraft-nightly"
mv "$bin/vectorcraft-cli" "$bin/vectorcraft-nightly-cli"

test ! -e "$stage/bin/.vectorcraft-nightly"
cmp "$stage/bin/vectorcraft" "$bin/.vectorcraft-nightly/vectorcraft"
cmp "$stage/bin/vectorcraft-cli" "$bin/vectorcraft-nightly-cli"
export HOME="$work/home with space"
unset XDG_DATA_HOME VECTORCRAFT_CONFIG_DIR
expect() {
  expected="$1"
  shift
  actual=$(PATH="$bin:$PATH" vectorcraft-nightly "$@")
  if [[ "$actual" != "$expected" ]]; then
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  fi
}
expect "CONFIG=<$HOME/.local/share/vectorcraft-nightly>"
export XDG_DATA_HOME="$work/xdg with space"
expect "$(printf 'CONFIG=<%s>\n<%s>\n<%s>\n' "$XDG_DATA_HOME/vectorcraft-nightly" 'drawing with spaces.svg' '--other-name.svg')" \
  'drawing with spaces.svg' '--other-name.svg'
expect "$(printf 'CONFIG=<>\n<--version>\n')" --version
expect "$(printf 'CONFIG=<>\n<-V>\n')" -V
VECTORCRAFT_CONFIG_DIR="$work/explicit with space" \
  expect "$(printf 'CONFIG=<%s>\n<%s>\n' "$work/explicit with space" 'sample.svg')" sample.svg
VECTORCRAFT_CONFIG_DIR="" \
  expect "$(printf 'CONFIG=<>\n<%s>\n' 'sample.svg')" sample.svg

echo "PASS: Arch Nightly handoff isolates the profile and preserves launch arguments"
