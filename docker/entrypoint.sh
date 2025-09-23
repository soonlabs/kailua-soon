#!/bin/bash

set -e

# Function to show usage
show_usage() {
    echo "Usage: $0 <command> [options]"
    echo ""
    echo "Available commands:"
    echo "  propose     - Run kailua-cli propose with specified options"
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
    echo "Examples:"
    echo "  $0 propose"
    echo "  $0 --help"
    echo "  $0 sync --config /path/to/config"
}

# Default values
DATA_DIR=${DATA_DIR:-/data}

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
