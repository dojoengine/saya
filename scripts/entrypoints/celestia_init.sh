#!/bin/bash

set -e

if [ -f "/home/celestia/initialized" ]; then
    echo "Node already initialized. Running config update..."

    celestia light config-update --core.ip $RPC_URL --p2p.network $NETWORK
else
    echo "Initializing node..."

    celestia light init --p2p.network $NETWORK
    touch /home/celestia/initialized

    celestia light auth admin --p2p.network $NETWORK
fi
