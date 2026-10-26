#!/usr/bin/env bash
if [ "$1" = "--version" ]; then
    echo "0.29.2"
    exit 0
fi
if [ "$1" = "--modversion" ]; then
    echo "1.2.9"
    exit 0
fi
if [ "$1" = "--atleast-pkgconfig-version" ] || [ "$1" = "--atleast-version" ]; then
    exit 0
fi
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARCH="${SENTINEL_ARCH:-armv7}"
echo "-L$DIR/../libs/$ARCH -lasound"
exit 0
