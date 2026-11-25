#!/bin/sh
# Install the Desmo `ik` runtime binary from a versioned GitHub Release.
# Verifies SHA256SUMS before installing. No `gh` CLI or registry required.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/GauravAlbal/desmo/main/install.sh | sh
#   DESMO_VERSION=v0.5.1 DESMO_INSTALL_DIR=/usr/local/bin sh install.sh
set -eu

REPO="${DESMO_REPO:-GauravAlbal/desmo}"
VERSION="${DESMO_VERSION:-v0.5.1}"
INSTALL_DIR="${DESMO_INSTALL_DIR:-$HOME/.local/bin}"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS:$ARCH" in
  Darwin:arm64|Darwin:aarch64)
    ASSET="ik-darwin-arm64.tar.gz"
    ;;
  Linux:x86_64|Linux:amd64)
    ASSET="ik-linux-amd64.tar.gz"
    ;;
  *)
    echo "Desmo prebuilt binaries currently support macOS arm64 and Linux x86_64 (detected $OS/$ARCH)." >&2
    echo "Build from source instead: cargo install --git https://github.com/$REPO --tag $VERSION --bin ik" >&2
    exit 1
    ;;
esac

BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
TMPDIR_WORK="$(mktemp -d "${TMPDIR:-/tmp}/desmo-install.XXXXXX")"
trap 'rm -rf "$TMPDIR_WORK"' EXIT INT TERM

echo "Downloading Desmo $VERSION ($ASSET)..."
curl -fsSL "$BASE_URL/$ASSET" -o "$TMPDIR_WORK/$ASSET"
curl -fsSL "$BASE_URL/SHA256SUMS" -o "$TMPDIR_WORK/SHA256SUMS"

EXPECTED="$(awk -v file="$ASSET" '$2 == file || $2 == "*" file { print $1; exit }' "$TMPDIR_WORK/SHA256SUMS")"
if [ -z "$EXPECTED" ]; then
  echo "error: $ASSET not found in SHA256SUMS for $VERSION" >&2
  exit 1
fi

if command -v shasum >/dev/null 2>&1; then
  ACTUAL="$(shasum -a 256 "$TMPDIR_WORK/$ASSET" | awk '{ print $1 }')"
elif command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$TMPDIR_WORK/$ASSET" | awk '{ print $1 }')"
else
  echo "error: neither shasum nor sha256sum found; refusing unverified install" >&2
  exit 1
fi

if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "error: SHA-256 mismatch for $ASSET (expected $EXPECTED, got $ACTUAL)" >&2
  exit 1
fi

tar -xzf "$TMPDIR_WORK/$ASSET" -C "$TMPDIR_WORK"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$TMPDIR_WORK/ik" "$INSTALL_DIR/ik"

echo "Installed: $INSTALL_DIR/ik"
"$INSTALL_DIR/ik" --version

case ":${PATH:-}:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "Add $INSTALL_DIR to your PATH:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac
