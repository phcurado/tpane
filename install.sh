#!/usr/bin/env sh
# install.sh — download the latest castr release binary into ~/.local/bin.
set -eu

REPO="phcurado/castr"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)
case "$arch" in
  x86_64|amd64) arch=amd64 ;;
  aarch64|arm64) arch=arm64 ;;
  *) echo "unsupported arch: $arch" >&2; exit 1 ;;
esac
case "$os" in
  linux|darwin) ;;
  *) echo "unsupported os: $os" >&2; exit 1 ;;
esac

if [ "${VERSION:-}" ]; then
  tag="$VERSION"
else
  tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
fi
if [ -z "$tag" ]; then
  echo "could not resolve latest tag" >&2
  exit 1
fi
case "$tag" in
  v*) ;;
  *) tag="v$tag" ;;
esac

asset="castr_${tag#v}_${os}_${arch}.tar.gz"
url="https://github.com/$REPO/releases/download/$tag/$asset"

echo "downloading $url"
curl -fsSL "$url" -o "$TMP/$asset"
curl -fsSL "https://github.com/$REPO/releases/download/$tag/checksums.txt" -o "$TMP/checksums.txt"
expected=$(grep "  $asset\$" "$TMP/checksums.txt" | awk '{print $1}')
if [ -z "$expected" ]; then
  echo "checksum not found for $asset" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$TMP/$asset" | awk '{print $1}')
else
  actual=$(shasum -a 256 "$TMP/$asset" | awk '{print $1}')
fi
if [ "$actual" != "$expected" ]; then
  echo "checksum mismatch for $asset" >&2
  exit 1
fi
tar -xzf "$TMP/$asset" -C "$TMP"

mkdir -p "$BIN_DIR"
install -m 0755 "$TMP/castr" "$BIN_DIR/castr"

echo "installed: $BIN_DIR/castr ($tag)"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "note: $BIN_DIR is not in PATH" >&2 ;;
esac
