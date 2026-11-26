#!/usr/bin/env bash
# Build and package VectorCraft for FreeBSD:
#
#   $DIST/vectorcraft-<version>-freebsd-<arch>.tar.gz   a /usr/local-style tree:
#       vectorcraft-<version>-freebsd-<arch>/{bin, share/applications, share/icons, share/mime,
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
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# FreeBSD calls x86_64 "amd64"; release names use the Rust/Linux spelling.
case "$(uname -m)" in
  amd64 | x86_64) HOST_ARCH=x86_64 ;;
  arm64 | aarch64) HOST_ARCH=aarch64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 2 ;;
esac
# The release workflow cross-compiles aarch64 on x86_64 by setting CARGO_BUILD_TARGET; the arch follows the target.
ARCH="$HOST_ARCH"
BIN="$CARGO_TARGET_DIR/release"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  case "$CARGO_BUILD_TARGET" in
    aarch64-unknown-freebsd) ARCH=aarch64 ;;
    *) ARCH=x86_64 ;;
  esac
  BIN="$CARGO_TARGET_DIR/$CARGO_BUILD_TARGET/release"
fi
BASENAME="vectorcraft-$VERSION-freebsd-$ARCH"

echo "==> VectorCraft $VERSION for FreeBSD $ARCH"

# The man-and-completions example writes vectorcraft-cli's man pages and shell completions below. It
# is built in its own cargo run: built with the binaries, its dev-dependencies' features would be
# unified into them. A cross build can't run it.
if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p vectorcraft -p vectorcraft-cli ${CARGO_BUILD_TARGET:+--target "$CARGO_BUILD_TARGET"})
  if [[ "$ARCH" = "$HOST_ARCH" ]]; then
    (cd "$ROOT" && cargo build --release --locked -p vectorcraft-cli --example man-and-completions ${CARGO_BUILD_TARGET:+--target "$CARGO_BUILD_TARGET"})
  fi
fi
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
  if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$STAGE/share/doc/vectorcraft/"; fi
done
# vectorcraft-cli's man pages and shell completions, from its clap definition, written by the
# example built above. A cross build can't run it, and a --skip-build run whose prebuilt binaries
# came without the example has none: both leave them out.
CLI_DOCS="$BIN/examples/man-and-completions"
if [[ "$ARCH" != "$HOST_ARCH" ]]; then
  warn "cross-building $ARCH, so vectorcraft-cli's man pages and completions are left out"
elif [[ ! -x "$CLI_DOCS" ]]; then
  warn "$CLI_DOCS is not built, so vectorcraft-cli's man pages and completions are left out"
else
  "$CLI_DOCS" "$WORK/cli-docs"
  mkdir -p "$STAGE/man/man1" "$STAGE/share/bash-completion/completions" \
    "$STAGE/share/zsh/site-functions" "$STAGE/share/fish/vendor_completions.d"
  install -m 644 "$WORK"/cli-docs/*.1 "$STAGE/man/man1/"
  install -m 644 "$WORK/cli-docs/vectorcraft-cli.bash" "$STAGE/share/bash-completion/completions/vectorcraft-cli"
  install -m 644 "$WORK/cli-docs/_vectorcraft-cli" "$STAGE/share/zsh/site-functions/_vectorcraft-cli"
  install -m 644 "$WORK/cli-docs/vectorcraft-cli.fish" "$STAGE/share/fish/vendor_completions.d/vectorcraft-cli.fish"
fi

# ---- .tar.gz -------------------------------------------------------------------------------------
tar -C "$WORK" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $DIST/$BASENAME.tar.gz"

# A cross-built binary can't run on this host.
if [[ "$ARCH" = "$HOST_ARCH" ]]; then "$STAGE/bin/vectorcraft-cli" --version; fi
echo "==> done"
ls -lh "$DIST"
