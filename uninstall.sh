#!/bin/sh
set -eu

PREFIX="${WEBYLIB_PREFIX:-$HOME/.local}"

if [ -f "${PREFIX}/bin/webyc" ]; then
    rm "${PREFIX}/bin/webyc"
    echo "Removed ${PREFIX}/bin/webyc"
else
    echo "webyc not found at ${PREFIX}/bin/webyc"
fi

echo "Note: Wallet data (*.db files) is not removed."
