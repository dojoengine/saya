#!/bin/bash

set -e

if [ -z "$CAIRO_VERSION" ]; then
  echo "CAIRO_VERSION not set" >&2
  exit 1
fi

CAIRO_VERSION=${CAIRO_VERSION#"v"}

if [ "$CAIRO_VERSION" = "0.13.2.1" ]; then
  # Doing this as v0.13.2.1 is not tagged
  git clone --recursive https://github.com/starkware-libs/cairo-lang -b v0.13.3 --depth 2 /src
  cd /src && git checkout a86e92bfde9c171c0856d7b46580c66e004922f3
else
  git clone --recursive https://github.com/starkware-libs/cairo-lang -b v$CAIRO_VERSION --depth 1 /src
fi

cd /src/src && cairo-compile --no_debug_info --output /output/snos.json starkware/starknet/core/os/os.cairo
