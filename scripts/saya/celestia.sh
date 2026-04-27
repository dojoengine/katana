#!/usr/bin/env bash

# Uses Docker to manage a Celestia light node locally for testing purposes.
# Meant to be used once for initialization and then starting the node.
# If the light node is not fully synced, the init may be run again to update the config.
#
# Usage:
# To init / update the config:
# ./scripts/celestia.sh init
#
# To start the node (not in detached mode, modify at your will for the usage you need):
# ./scripts/celestia.sh

set -e

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

export NETWORK=mocha                                               
export RPC_URL=rpc-mocha.pops.one
export RPC_PORT=9090
export IMAGE=ghcr.io/celestiaorg/celestia-node:v0.28.4
export VOLUME=celestia-light-mocha

if [ "$1" = "init" ]; then
    $Sudo docker run --rm -e NETWORK=$NETWORK -e RPC_URL=$RPC_URL -e RPC_PORT=$RPC_PORT \
        -v $VOLUME:/home/celestia \
        -v "${SCRIPT_DIR}/entrypoints/celestia_init.sh:/entry:ro" \
        --entrypoint "/entry" \
        $IMAGE

        exit 0
fi

$SUDO docker run --rm -e NODE_TYPE=light -e P2P_NETWORK=$NETWORK \
    -v $VOLUME:/home/celestia \
    -p 127.0.0.1:26658:26658 \
    $IMAGE \
    celestia light start --core.ip $RPC_URL --core.port $RPC_PORT --p2p.network $NETWORK
