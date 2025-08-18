#!/bin/bash

# Script to verify contract deployment status and key relationships
set -e

# Color definitions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# L1 node configuration (can be overridden with environment variables)
L1_RPC_URL="${L1_RPC_URL:-http://localhost:8545}"
L1_CHAIN_ID="11155111"

echo -e "${BLUE}🔍 Verify contract deployment status and key relationships${NC}"
echo "=================================="

# Check if L1 node is running
echo -e "${YELLOW}1. Check L1 node connection...${NC}"
if curl -s -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' $L1_RPC_URL > /dev/null 2>&1; then
    echo -e "${GREEN}✅ L1 node connection OK${NC}"
else
    echo -e "${RED}❌ Unable to connect to L1 node, please ensure node is running${NC}"
    echo "Please check: docker-compose up -d l1"
    exit 1
fi

# Get chain ID for verification
CHAIN_ID_HEX=$(curl -s -X POST -H "Content-Type: application/json" --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' $L1_RPC_URL | jq -r '.result')
CHAIN_ID=$(printf "%d" $((16#${CHAIN_ID_HEX:2})))
echo "   Chain ID: $CHAIN_ID"

# Check contract address file
echo -e "\n${YELLOW}2. Check contract address file...${NC}"
if [ ! -f "devnet/addresses.json" ]; then
    echo -e "${RED}❌ addresses.json file does not exist${NC}"
    exit 1
fi

CONTRACT_COUNT=$(jq 'keys | length' devnet/addresses.json)
echo -e "${GREEN}✅ Found $CONTRACT_COUNT contract addresses${NC}"

# Check private key file
echo -e "\n${YELLOW}3. Check private key file...${NC}"
if [ ! -f "devnet/genesis-keys.json" ]; then
    echo -e "${RED}❌ genesis-keys.json file does not exist${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Private key file exists${NC}"

# Verify contract deployment status
echo -e "\n${YELLOW}4. Verify contract deployment status...${NC}"
DEPLOYED_COUNT=0
FAILED_COUNT=0
# Don't exit on single errors in the verification loop
set +e

USE_CAST=false
if command -v cast >/dev/null 2>&1; then
  USE_CAST=true
  echo -e "${BLUE}ℹ️  Using cast to check contract code${NC}"
else
  echo -e "${YELLOW}⚠️  cast not found, falling back to JSON-RPC check${NC}"
fi

for contract in $(jq -r 'keys[]' devnet/addresses.json); do
    address=$(jq -r ".$contract" devnet/addresses.json)

    if $USE_CAST; then
        code=$(cast code "$address" --rpc-url "$L1_RPC_URL" 2>/dev/null | tr -d '\n' || true)
    else
        code=$(curl -s -X POST -H "Content-Type: application/json" \
            --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getCode\",\"params\":[\"$address\",\"latest\"],\"id\":1}" \
            "$L1_RPC_URL" | jq -r '.result')
    fi

    if [ -n "$code" ] && [ "$code" != "0x" ] && [ "$code" != "null" ]; then
        # 计算代码大小（字节数）
        code_len=${#code}
        if [ $code_len -ge 2 ]; then
          bytes=$(( (code_len - 2) / 2 ))
        else
          bytes=0
        fi
        echo -e "  ${GREEN}✅ $contract: $address (${bytes} bytes)${NC}"
        ((DEPLOYED_COUNT++))
    else
        echo -e "  ${RED}❌ $contract: $address (not deployed)${NC}"
        ((FAILED_COUNT++))
    fi
done

# Restore strict mode
set -e

echo -e "\n${BLUE}Deployment statistics:${NC}"
echo "  ✅ Deployed: $DEPLOYED_COUNT"
echo "  ❌ Not deployed: $FAILED_COUNT"

# Exit with non-zero status if there are undeployed contracts for CI alerts
if [ "$FAILED_COUNT" -gt 0 ]; then
  exit 1
fi

# Verify proposer / batcher are correctly configured in storage slots
echo -e "\n${YELLOW}5. Verify proposer / batcher configuration...${NC}"

# Read expected addresses from key file
GS_ADMIN_ADDR=$(jq -r '.gs_admin.address' devnet/genesis-keys.json)
GS_ADMIN_PRIV=$(jq -r '.gs_admin.private_key' devnet/genesis-keys.json)
GS_BATCHER_ADDR=$(jq -r '.gs_batcher.address' devnet/genesis-keys.json)
GS_PROPOSER_ADDR=$(jq -r '.gs_proposer.address' devnet/genesis-keys.json)

# Read proxy addresses from address file
L2OO_PROXY=$(jq -r '.L2OutputOracleProxy' devnet/addresses.json)
SYSCFG_PROXY=$(jq -r '.SystemConfigProxy' devnet/addresses.json)

# Helper function: extract 20-byte address from 32-byte storage value
hex_to_address() {
  local word="$1" # 0x + 64 hex chars
  local trimmed=${word#0x}
  local last40=${trimmed: -40}
  echo "0x${last40}" | tr '[:upper:]' '[:lower:]'
}

# Helper function: read storage slot of an address (prefer cast, fallback to JSON-RPC)
read_storage() {
  local addr="$1"
  local slot="$2"
  local value=""
  if command -v cast >/dev/null 2>&1; then
    value=$(cast storage "$addr" "$slot" --rpc-url "$L1_RPC_URL" 2>/dev/null || true)
  fi
  if [ -z "$value" ]; then
    value=$(curl -s -X POST -H "Content-Type: application/json" \
      --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getStorageAt\",\"params\":[\"$addr\",\"$(printf "0x%x" "$slot")\",\"latest\"],\"id\":1}" \
      "$L1_RPC_URL" | jq -r '.result')
  fi
  echo "$value"
}

normalize_addr() {
  echo "$1" | tr '[:upper:]' '[:lower:]'
}

EXPECTED_PROPOSER=$(normalize_addr "$GS_PROPOSER_ADDR")
EXPECTED_BATCHER=$(normalize_addr "$GS_BATCHER_ADDR")

FAILURES=0

echo -e "  L2OutputOracleProxy: $L2OO_PROXY | slot 7 (proposer)"
val_proposer=$(read_storage "$L2OO_PROXY" 7)
addr_proposer=$(hex_to_address "$val_proposer")
echo -e "    Read proposer: $addr_proposer"
if [ "$addr_proposer" = "$EXPECTED_PROPOSER" ]; then
  echo -e "    ${GREEN}✅ proposer matches${NC}"
else
  echo -e "    ${RED}❌ proposer mismatch${NC}"
  echo -e "      Expected: $EXPECTED_PROPOSER"
  echo -e "      Actual: $addr_proposer (raw: $val_proposer)"
  FAILURES=$((FAILURES+1))
fi

echo -e "  SystemConfigProxy: $SYSCFG_PROXY | slot 103 (batcher)"
val_batcher=$(read_storage "$SYSCFG_PROXY" 103)
addr_batcher=$(hex_to_address "$val_batcher")
echo -e "    Read batcher: $addr_batcher"
if [ "$addr_batcher" = "$EXPECTED_BATCHER" ]; then
  echo -e "    ${GREEN}✅ batcher matches${NC}"
else
  echo -e "    ${RED}❌ batcher mismatch${NC}"
  echo -e "      Expected: $EXPECTED_BATCHER"
  echo -e "      Actual: $addr_batcher (raw: $val_batcher)"
  FAILURES=$((FAILURES+1))
fi

if [ "$FAILURES" -gt 0 ]; then
  echo -e "\n${RED}❌ proposer / batcher configuration verification failed ($FAILURES items)${NC}"
  exit 1
else
  echo -e "\n${GREEN}✅ proposer / batcher configuration verification passed${NC}"
fi

##########
# 6. Check account balances (should be 10000 ETH)
##########
echo -e "\n${YELLOW}6. Check account balances (should be 10000 ETH)...${NC}"

EXPECT_BALANCE_ETH="${EXPECT_BALANCE_ETH:-10000}"
EXPECT_MODE="${EXPECT_MODE:-ge}" # ge: at least, eq: equal

TARGET_WEI="$EXPECT_BALANCE_ETH"
if command -v cast >/dev/null 2>&1; then
  TARGET_WEI=$(cast to-wei "$EXPECT_BALANCE_ETH" ether)
else
  # 10000 * 1e18
  if [ "$EXPECT_BALANCE_ETH" = "10000" ]; then
    TARGET_WEI="10000000000000000000000"
  else
    TARGET_WEI=$(python3 - "$EXPECT_BALANCE_ETH" <<'PY'
import sys, decimal
eth = decimal.Decimal(sys.argv[1])
wei = eth * decimal.Decimal(10**18)
print(int(wei))
PY
)
  fi
fi

# Fixed common test accounts
declare -a CHECK_ADDRS=(
  "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"  # Hardhat account 0
  "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"  # Hardhat account 1
  "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"  # Hardhat account 2
  "0x90F79bf6EB2c4f870365E785982E1f101E93b906"  # Common test account 1
  "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65"  # Common test account 2
  "0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc"  # Common test account 3
)

# Add all addresses from genesis-keys.json
for key in gs_admin gs_batcher gs_proposer gs_sequencer deployer; do
  addr=$(jq -r ".$key.address" devnet/genesis-keys.json 2>/dev/null || echo "")
  if [ -n "$addr" ] && [ "$addr" != "null" ]; then
    CHECK_ADDRS+=("$addr")
  fi
done

# Remove duplicates
mapfile -t CHECK_ADDRS < <(printf "%s\n" "${CHECK_ADDRS[@]}" | awk 'NF' | awk '{print tolower($0)}' | sort -u)

BALANCE_FAIL=0

if ! command -v cast >/dev/null 2>&1; then
  echo -e "${YELLOW}⚠️  cast command not available, skipping balance check${NC}"
else
  for addr in "${CHECK_ADDRS[@]}"; do
    bal_wei=$(cast balance "$addr" --rpc-url "$L1_RPC_URL" 2>/dev/null | tr -d '\n' || echo "")

    if [ -z "$bal_wei" ]; then
      echo -e "  ${RED}❌ $addr: unable to get balance${NC}"
      BALANCE_FAIL=$((BALANCE_FAIL+1))
      continue
    fi

  # Large integer comparison (using Python, supports ge/eq)
  cmp=$(python3 - "$bal_wei" "$TARGET_WEI" "$EXPECT_MODE" <<'PY'
import sys
bal=int(sys.argv[1]); target=int(sys.argv[2]); mode=sys.argv[3]
ok = (bal==target) if mode=='eq' else (bal>=target)
print('OK' if ok else 'NG')
PY
)

    if [ "$cmp" = "OK" ]; then
      disp="${EXPECT_BALANCE_ETH}"
      [ "$EXPECT_MODE" = "ge" ] && disp=">=${EXPECT_BALANCE_ETH}"
      echo -e "  ${GREEN}✅ $addr: meets ${disp} ETH${NC}"
    else
      # Use cast from-wei to display
      eth_display=$(cast from-wei "$bal_wei" ether 2>/dev/null || echo "")
      need="${EXPECT_BALANCE_ETH}"
      [ "$EXPECT_MODE" = "ge" ] && need=">= ${EXPECT_BALANCE_ETH}"
      echo -e "  ${RED}❌ $addr: $eth_display ETH (should be ${need})${NC}"
      BALANCE_FAIL=$((BALANCE_FAIL+1))
    fi
  done
fi

if [ "$BALANCE_FAIL" -gt 0 ]; then
  echo -e "\n${RED}❌ Balance verification failed: $BALANCE_FAIL addresses not 10000 ETH${NC}"
  exit 1
fi

echo -e "\n${GREEN}✅ Verification complete!${NC}"
