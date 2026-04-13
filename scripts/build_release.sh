#!/bin/sh
set -eu

VERSION="${1:?Usage: build_release.sh VERSION [PLATFORM]}"
PLATFORM="${2:-}"

if [ -z "$PLATFORM" ]; then
    OS="$(uname -s)"
    ARCH="$(uname -m)"
    case "$OS" in
        Linux)  PLATFORM="Linux-${ARCH}" ;;
        Darwin) PLATFORM="macOS-${ARCH}" ;;
        MINGW*|MSYS*|CYGWIN*) PLATFORM="Windows-x86_64" ;;
        FreeBSD) PLATFORM="FreeBSD-${ARCH}" ;;
        *) PLATFORM="${OS}-${ARCH}" ;;
    esac
fi

BINARY="target/release/webyc"
if [ ! -f "$BINARY" ]; then
    echo "Error: $BINARY not found. Run 'cargo build --release' first."
    exit 1
fi

STAGING="webylib-${VERSION}-${PLATFORM}"
mkdir -p "$STAGING"

cp "$BINARY" "$STAGING/"
cp LICENSE README.md CHANGELOG.md "$STAGING/"

# Include libraries if present
for ext in a so dylib dll lib; do
    find target/release -maxdepth 1 -name "*.${ext}" -exec cp {} "$STAGING/" \; 2>/dev/null || true
done

# Include C header if present
if [ -d include ]; then
    cp -r include "$STAGING/"
fi

tar czf "${STAGING}.tar.gz" "$STAGING"
shasum -a 256 "${STAGING}.tar.gz" > "${STAGING}.tar.gz.sha256"

echo "Created: ${STAGING}.tar.gz"
cat "${STAGING}.tar.gz.sha256"

rm -rf "$STAGING"
