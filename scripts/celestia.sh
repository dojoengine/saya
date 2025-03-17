#!/usr/bin/env bash

# Uses Docker to manage a Celestia light node locally for testing purposes.

set -e

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

export NETWORK=mocha                                               
export RPC_URL=rpc-mocha.pops.one
export RPC_PORT=9090
export IMAGE=ghcr.io/celestiaorg/celestia-node:v0.21.8-mocha
export VOLUME=celestia-light-mocha

$SUDO docker run --rm -e NETWORK=$NETWORK -e RPC_URL=$RPC_URL -e RPC_PORT=$RPC_PORT \
    -v $VOLUME:/home/celestia \
    -v "${SCRIPT_DIR}/entrypoints/celestia_init.sh:/entry:ro" \
    --entrypoint "/entry" \
    $IMAGE

$SUDO docker run --rm \
    -v $VOLUME:/home/celestia \
    --network=host \
    $IMAGE \
    celestia light start --core.ip $RPC_URL --core.port $RPC_PORT --p2p.network $NETWORK
