#!/bin/sh
set -eu

REPO="webycash/webylib"
PREFIX="${WEBYLIB_PREFIX:-$HOME/.local}"
VERSION="${WEBYLIB_VERSION:-}"

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux)  PLATFORM="Linux-${ARCH}" ;;
    Darwin) PLATFORM="macOS-${ARCH}" ;;
    FreeBSD) PLATFORM="FreeBSD-${ARCH}" ;;
    *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

if [ -z "$VERSION" ]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"v\(.*\)".*/\1/')"
fi

TARBALL="webylib-${VERSION}-${PLATFORM}.tar.gz"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${TARBALL}"

echo "Downloading webylib v${VERSION} for ${PLATFORM}..."
curl -fsSL "$URL" -o "$TARBALL"
curl -fsSL "${URL}.sha256" -o "${TARBALL}.sha256"

echo "Verifying checksum..."
shasum -a 256 -c "${TARBALL}.sha256"

echo "Installing to ${PREFIX}/bin/..."
mkdir -p "${PREFIX}/bin"
tar xzf "$TARBALL"
cp "webylib-${VERSION}-${PLATFORM}/webyc" "${PREFIX}/bin/"
chmod +x "${PREFIX}/bin/webyc"

rm -rf "$TARBALL" "${TARBALL}.sha256" "webylib-${VERSION}-${PLATFORM}"

echo "Installed webyc to ${PREFIX}/bin/webyc"

mkdir -p "$HOME/.webyc"
echo "Created default wallet directory: $HOME/.webyc"

case ":$PATH:" in
    *":${PREFIX}/bin:"*) ;;
    *) echo "Add ${PREFIX}/bin to your PATH: export PATH=\"${PREFIX}/bin:\$PATH\"" ;;
esac
