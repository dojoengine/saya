#!/usr/bin/env bash

# Uses Docker to deterministically generate the `layout_bridge` program artifact.
#
# In environments that need `sudo` to run `docker` commands, set the `SUDO` variable to `sudo`:
#
# $ SUDO=sudo ./generate_layout_bridge.sh

set -e

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
REPO_ROOT=$( dirname -- $SCRIPT_DIR )

CAIRO_VERSION="0.13.3"
COMPILER_VERSION="0.13.2"

mkdir -p $REPO_ROOT/programs

$SUDO docker run -it --rm \
  -v "$REPO_ROOT/programs:/output" \
  -v "$SCRIPT_DIR/entrypoints/layout_bridge.sh:/entry:ro" \
  -e "CAIRO_VERSION=$CAIRO_VERSION" \
  --entrypoint "/entry" \
  starknet/cairo-lang:$COMPILER_VERSION

$SUDO docker run --rm \
  -v "$REPO_ROOT/programs:/output" \
  --user root \
  tmknom/prettier:3.2.5 \
  --write "/output/layout_bridge.json"
