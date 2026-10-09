#!/usr/bin/env bash
# Build and package Vector W3K2 for FreeBSD:
#
#   $DIST/vectorcraft-<version>-freebsd-x86_64.tar.gz   a /usr/local-style tree:
#       vectorcraft-<version>-freebsd-x86_64/{bin, share/applications, share/icons, share/mime,
#       share/metainfo, share/doc/vectorcraft}
#
# Install by copying the tree's contents into /usr/local, e.g.
#   tar -xzf vectorcraft-*-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
#
# Usage: packaging/freebsd/package.sh [--skip-build]
#
# Needs: bash, cargo and the build dependencies listed in .github/workflows/freebsd.yml. FreeBSD's
# install(1) has no -D, so directories are created first.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
LINUX="$ROOT/packaging/linux"
APP_ID=ai.storyteller.vectorcraft

SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# FreeBSD calls x86_64 "amd64"; release names use the Rust/Linux spelling.
case "$(uname -m)" in
  amd64 | x86_64) ARCH=x86_64 ;;
  arm64 | aarch64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 2 ;;
esac
BASENAME="vectorcraft-$VERSION-freebsd-$ARCH"

echo "==> Vector W3K2 $VERSION for FreeBSD $ARCH"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p vectorcraft -p vectorcraft-cli)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$BASENAME"
rm -rf "$WORK"

# ---- stage a /usr/local-style tree ---------------------------------------------------------------
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/mime/packages" "$STAGE/share/metainfo" \
  "$STAGE/share/icons" "$STAGE/share/doc/vectorcraft"
install -m 755 "$BIN/vectorcraft" "$BIN/vectorcraft-cli" "$STAGE/bin/"
strip "$STAGE/bin/vectorcraft" "$STAGE/bin/vectorcraft-cli" 2>/dev/null || true
# The desktop entry, MIME type and metainfo are the freedesktop files the Linux packages use.
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/$APP_ID.desktop"
install -m 644 "$LINUX/$APP_ID.mime.xml" "$STAGE/share/mime/packages/$APP_ID.xml"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$VECTORCRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
copy_docs "$STAGE/share/doc/vectorcraft"
for f in NOTICE ASSETS.md; do
  if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$STAGE/share/doc/vectorcraft/"; fi
done

# ---- .tar.gz -------------------------------------------------------------------------------------
tar -C "$WORK" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $DIST/$BASENAME.tar.gz"

"$STAGE/bin/vectorcraft-cli" --version
echo "==> done"
ls -lh "$DIST"
