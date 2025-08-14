#!/bin/bash
set -exu

VERBOSITY=${GETH_VERBOSITY:-3}
GETH_DATA_DIR=/db
GETH_CHAINDATA_DIR="$GETH_DATA_DIR/geth/chaindata"
GENESIS_FILE_PATH="${GENESIS_FILE_PATH:-/genesis.json}"

RPC_PORT="${RPC_PORT:-8545}"
WS_PORT="${WS_PORT:-8546}"

# Optional auto-mining controls
# If AUTO_MINE=true, start geth with mining flags. This helps private devnets auto-produce blocks.
AUTO_MINE=${AUTO_MINE:-false}
MINER_COINBASE=${MINER_COINBASE:-0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266}

# Optional dev chain (instant sealing, no beacon, ignores custom genesis)
GETH_DEV=${GETH_DEV:-false}
GETH_DEV_PERIOD=${GETH_DEV_PERIOD:-1}

if [ ! -d "$GETH_CHAINDATA_DIR" ]; then
    echo "$GETH_CHAINDATA_DIR missing, running init"
    echo "Initializing genesis."
    geth --verbosity="$VERBOSITY" init \
        --datadir="$GETH_DATA_DIR" \
        --state.scheme=hash \
        "$GENESIS_FILE_PATH"
else
    echo "$GETH_CHAINDATA_DIR exists."
fi

# Warning: Archive mode is required, otherwise old trie nodes will be
# pruned within minutes of starting the devnet.

exec geth \
	--datadir="$GETH_DATA_DIR" \
	--verbosity="$VERBOSITY" \
    $(if [ "$GETH_DEV" = "true" ]; then echo --dev --dev.period "$GETH_DEV_PERIOD"; fi) \
	--http \
	--http.corsdomain="*" \
	--http.vhosts="*" \
	--http.addr=0.0.0.0 \
	--http.port="$RPC_PORT" \
	--http.api=web3,debug,eth,txpool,net,engine \
	--ws \
	--ws.addr=0.0.0.0 \
	--ws.port="$WS_PORT" \
	--ws.origins="*" \
	--ws.api=debug,eth,txpool,net,engine \
	--syncmode=full \
	--nodiscover \
	--maxpeers=1 \
	--rpc.allow-unprotected-txs \
	--authrpc.addr="0.0.0.0" \
	--authrpc.port="8551" \
	--authrpc.vhosts="*" \
	--authrpc.jwtsecret=/config/jwt-secret.txt \
	--gcmode=archive \
  --state.scheme=hash \
	--metrics \
	--metrics.addr=0.0.0.0 \
	--metrics.port=6060 \
	$(if [ "$AUTO_MINE" = "true" ]; then echo --mine --miner.etherbase "$MINER_COINBASE" --miner.threads 1 --miner.gasprice 0; fi) \
	"$@"
