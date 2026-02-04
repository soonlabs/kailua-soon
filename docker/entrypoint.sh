#!/bin/bash

set -e

# Function to show usage
show_usage() {
    echo "Usage: $0 <command> [options]"
    echo ""
    echo "Available commands:"
    echo "  propose     - Run kailua-cli propose with specified options"
    echo "  validate    - Run kailua-cli validate with Boundless proving"
    echo "  <any>       - Run kailua-cli with any arguments"
    echo ""
    echo "For propose command, the following environment variables are expected:"
    echo "  L1_RPC              - Ethereum RPC URL"
    echo "  L1_BEACON_RPC       - Ethereum Beacon RPC URL"
    echo "  L2_RPC              - Soon node URL"
    echo "  DA_PROXY            - DA proxy URL"
    echo "  DATA_DIR            - Data directory (default: /data)"
    echo "  PROPOSER            - Proposer key"
    echo "  VERBOSITY           - Verbosity level (optional)"
    echo ""
    echo "For validate command, the following environment variables are expected:"
    echo "  L1_RPC                  - Ethereum RPC URL"
    echo "  L1_BEACON_RPC           - Ethereum Beacon RPC URL"
    echo "  L2_RPC                  - Soon node URL"
    echo "  DA_PROXY                - DA proxy URL"
    echo "  DATA_DIR                - Data directory (default: /data)"
    echo "  VALIDATOR_KEY           - Validator private key"
    echo "  FAST_FORWARD_TARGET     - Fast forward target (optional)"
    echo "  BOUNDLESS_RPC_URL       - Boundless RPC URL (Base Mainnet)"
    echo "  BOUNDLESS_CHAIN_ID      - Boundless chain ID (default: 8453)"
    echo "  BOUNDLESS_WALLET_KEY    - Boundless wallet private key"
    echo "  BOUNDLESS_S3_ACCESS_KEY - S3 access key"
    echo "  BOUNDLESS_S3_SECRET_KEY - S3 secret key"
    echo "  BOUNDLESS_S3_BUCKET     - S3 bucket name"
    echo "  BOUNDLESS_S3_REGION     - S3 region"
    echo "  BOUNDLESS_S3_URL        - S3 URL"
    echo "  VERBOSITY               - Verbosity level (optional)"
    echo ""
    echo "Examples:"
    echo "  $0 propose"
    echo "  $0 validate"
    echo "  $0 --help"
    echo "  $0 sync --config /path/to/config"
}

# Default values
DATA_DIR=${DATA_DIR:-/data}
BOUNDLESS_CHAIN_ID=${BOUNDLESS_CHAIN_ID:-8453}

# Boundless contract addresses (Base Mainnet)
BOUNDLESS_MARKET_ADDRESS=${BOUNDLESS_MARKET_ADDRESS:-0xfd152dadc5183870710fe54f939eae3ab9f0fe82}
BOUNDLESS_VERIFIER_ROUTER_ADDRESS=${BOUNDLESS_VERIFIER_ROUTER_ADDRESS:-0x0b144e07a0826182b6b59788c34b32bfa86fb711}
BOUNDLESS_SET_VERIFIER_ADDRESS=${BOUNDLESS_SET_VERIFIER_ADDRESS:-0x1Ab08498CfF17b9723ED67143A050c8E8c2e3104}
BOUNDLESS_COLLATERAL_TOKEN_ADDRESS=${BOUNDLESS_COLLATERAL_TOKEN_ADDRESS:-0xaa61bb7777bd01b684347961918f1e07fbbce7cf}

# Boundless pricing defaults
BOUNDLESS_CYCLE_MIN_WEI=${BOUNDLESS_CYCLE_MIN_WEI:-20000}
BOUNDLESS_CYCLE_MAX_WEI=${BOUNDLESS_CYCLE_MAX_WEI:-500000}
BOUNDLESS_MEGA_CYCLE_COLLATERAL=${BOUNDLESS_MEGA_CYCLE_COLLATERAL:-1000000}

case "$1" in
    "propose")
        echo "Starting kailua-cli propose..."
        
        # Check required environment variables
        if [ -z "$L1_RPC" ] || [ -z "$L1_BEACON_RPC" ] || [ -z "$L2_RPC" ] || [ -z "$DA_PROXY" ] || [ -z "$PROPOSER" ]; then
            echo "Error: Missing required environment variables"
            echo "Required: L1_RPC, L1_BEACON_RPC, L2_RPC, DA_PROXY, PROPOSER"
            exit 1
        fi
        
        # Build the command
        CMD_ARGS=(
            "propose"
            "--eth-rpc-url" "$L1_RPC"
            "--beacon-rpc-url" "$L1_BEACON_RPC"
            "--soon-node-url" "$L2_RPC"
            "--da-proxy-url" "$DA_PROXY"
            "--data-dir" "$DATA_DIR"
            "--proposer-key" "$PROPOSER"
        )
        
        # Add verbosity if specified
        if [ -n "$VERBOSITY" ]; then
            CMD_ARGS+=("$VERBOSITY")
        fi
        
        echo "Executing: kailua-cli ${CMD_ARGS[*]}"
        exec kailua-cli "${CMD_ARGS[@]}"
        ;;

    "validate")
        echo "Starting kailua-cli validate..."
        
        # Check required environment variables
        if [ -z "$L1_RPC" ] || [ -z "$L1_BEACON_RPC" ] || [ -z "$L2_RPC" ] || [ -z "$DA_PROXY" ] || [ -z "$VALIDATOR_KEY" ]; then
            echo "Error: Missing required environment variables"
            echo "Required: L1_RPC, L1_BEACON_RPC, L2_RPC, DA_PROXY, VALIDATOR_KEY"
            exit 1
        fi
        
        # Check Boundless required environment variables
        if [ -z "$BOUNDLESS_RPC_URL" ] || [ -z "$BOUNDLESS_WALLET_KEY" ]; then
            echo "Error: Missing Boundless environment variables"
            echo "Required: BOUNDLESS_RPC_URL, BOUNDLESS_WALLET_KEY"
            exit 1
        fi
        
        # Check S3 required environment variables
        if [ -z "$BOUNDLESS_S3_ACCESS_KEY" ] || [ -z "$BOUNDLESS_S3_SECRET_KEY" ] || [ -z "$BOUNDLESS_S3_BUCKET" ] || [ -z "$BOUNDLESS_S3_REGION" ] || [ -z "$BOUNDLESS_S3_URL" ]; then
            echo "Error: Missing S3 environment variables"
            echo "Required: BOUNDLESS_S3_ACCESS_KEY, BOUNDLESS_S3_SECRET_KEY, BOUNDLESS_S3_BUCKET, BOUNDLESS_S3_REGION, BOUNDLESS_S3_URL"
            exit 1
        fi
        
        # Build the command
        CMD_ARGS=(
            "validate"
            "--eth-rpc-url" "$L1_RPC"
            "--beacon-rpc-url" "$L1_BEACON_RPC"
            "--soon-node-url" "$L2_RPC"
            "--da-proxy-url" "$DA_PROXY"
            "--data-dir" "$DATA_DIR"
            "--validator-key" "$VALIDATOR_KEY"
            # Boundless config
            "--boundless-rpc-url" "$BOUNDLESS_RPC_URL"
            "--boundless-chain-id" "$BOUNDLESS_CHAIN_ID"
            "--boundless-wallet-key" "$BOUNDLESS_WALLET_KEY"
            "--boundless-market-address" "$BOUNDLESS_MARKET_ADDRESS"
            "--boundless-verifier-router-address" "$BOUNDLESS_VERIFIER_ROUTER_ADDRESS"
            "--boundless-set-verifier-address" "$BOUNDLESS_SET_VERIFIER_ADDRESS"
            "--boundless-collateral-token-address" "$BOUNDLESS_COLLATERAL_TOKEN_ADDRESS"
            # S3 storage config
            "--storage-provider" "s3"
            "--s3-access-key" "$BOUNDLESS_S3_ACCESS_KEY"
            "--s3-secret-key" "$BOUNDLESS_S3_SECRET_KEY"
            "--s3-bucket" "$BOUNDLESS_S3_BUCKET"
            "--aws-region" "$BOUNDLESS_S3_REGION"
            "--s3-url" "$BOUNDLESS_S3_URL"
            # Boundless pricing
            "--boundless-cycle-min-wei" "$BOUNDLESS_CYCLE_MIN_WEI"
            "--boundless-cycle-max-wei" "$BOUNDLESS_CYCLE_MAX_WEI"
            "--boundless-mega-cycle-collateral" "$BOUNDLESS_MEGA_CYCLE_COLLATERAL"
        )
        
        # Add fast forward target if specified
        if [ -n "$FAST_FORWARD_TARGET" ]; then
            CMD_ARGS+=("--fast-forward-target" "$FAST_FORWARD_TARGET")
        fi
        
        # Add verbosity if specified
        if [ -n "$VERBOSITY" ]; then
            CMD_ARGS+=("$VERBOSITY")
        fi
        
        echo "Executing: kailua-cli ${CMD_ARGS[*]}"
        exec kailua-cli "${CMD_ARGS[@]}"
        ;;
        
    "--help" | "-h" | "help")
        show_usage
        exit 0
        ;;
        
    *)
        # For any other command, pass it directly to kailua-cli
        echo "Starting kailua-cli with arguments: $*"
        exec kailua-cli "$@"
        ;;
esac
