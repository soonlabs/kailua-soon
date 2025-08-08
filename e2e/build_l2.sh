#!/bin/bash

# Script to build L2 (Soon) related artifacts using soon/Makefile
set -euo pipefail

# Color definitions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Paths and defaults
E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Default SOON_ROOT uses a relative path to the sibling repo `../soon`
SOON_ROOT="${SOON_ROOT:-../soon}"
NETWORK_NAME="${NETWORK_NAME:-ethereum.testnet}"
L1_CHAIN_ID="${L1_CHAIN_ID:-11155111}"
L1_RPC_URL="${L1_RPC_URL:-http://localhost:8545}"

# All generated files should live under e2e directory
SOON_DATA_PATH="$E2E_DIR"

echo -e "${BLUE}🚀 Building L2 artifacts with Soon Makefile${NC}"
echo "=================================================="
echo -e "${YELLOW}📁 SOON_ROOT: ${SOON_ROOT}${NC}"
echo -e "${YELLOW}📁 SOON_DATA_PATH (outputs): ${SOON_DATA_PATH}${NC}"
echo -e "${YELLOW}🔗 L1 RPC: ${L1_RPC_URL}${NC}"
echo -e "${YELLOW}🔗 L1 Chain ID: ${L1_CHAIN_ID}${NC}"
echo -e "${YELLOW}🌐 Network: ${NETWORK_NAME}${NC}"

# Basic checks
if [ ! -d "${SOON_ROOT}" ]; then
  echo -e "${RED}❌ SOON_ROOT does not exist: ${SOON_ROOT}${NC}"
  exit 1
fi
if [ ! -f "${SOON_ROOT}/Makefile" ]; then
  echo -e "${RED}❌ Makefile not found under: ${SOON_ROOT}${NC}"
  exit 1
fi

# Prepare soon contract.json for Makefile genesis rule
ADDR_FILE="${E2E_DIR}/addresses.json"
if [ ! -f "${ADDR_FILE}" ]; then
  echo -e "${RED}❌ Missing ${ADDR_FILE}. Please run L1 build first.${NC}"
  exit 1
fi

# soon Makefile expects ${SOON_DATA_PATH}/${NETWORK_NAME}-contract.json to read SystemConfigProxy
CONTRACT_JSON="${SOON_DATA_PATH}/${NETWORK_NAME}-contract.json"
echo -e "${YELLOW}🔧 Writing ${CONTRACT_JSON} from e2e/addresses.json${NC}"
jq '{SystemConfigProxy:.SystemConfigProxy}' "${ADDR_FILE}" > "${CONTRACT_JSON}"

KEYS_SRC="${E2E_DIR}/genesis-keys.json"
KEYS_JSON="${SOON_DATA_PATH}/${NETWORK_NAME}-keys.json"
if [ ! -f "${KEYS_SRC}" ]; then
  echo -e "${RED}❌ Missing ${KEYS_SRC}. Please run L1 build first.${NC}"
  exit 1
fi

echo ""
echo -e "${YELLOW}🔧 Step 1: generate only required Soon keys (no EVM keys)${NC}"

# Ensure output directories exist under e2e per docker-compose
keypair_dir="${SOON_DATA_PATH}/.keypair"
mkdir -p "${keypair_dir}"
# Generate only Solana-style keys required by Soon genesis
solana-keygen new --no-bip39-passphrase -f -o "$keypair_dir/identity.json" >/dev/null
solana-keygen new --no-bip39-passphrase -f -o "$keypair_dir/upgrader.json" >/dev/null
solana-keygen new --no-bip39-passphrase -f -o "$keypair_dir/faucet.json" >/dev/null
solana-keygen new --no-bip39-passphrase -f -o "$keypair_dir/network_identity.json" >/dev/null

soon_identity_key=$(cat "$keypair_dir/identity.json")
soon_identity_pk=$(echo $soon_identity_key | solana-keygen pubkey -)
soon_upgrader_key=$(cat "$keypair_dir/upgrader.json")
soon_upgrader_pk=$(echo $soon_upgrader_key | solana-keygen pubkey -)
soon_faucet_key=$(cat "$keypair_dir/faucet.json")
soon_faucet_pk=$(echo $soon_faucet_key | solana-keygen pubkey -)
soon_network_identity_key=$(cat "$keypair_dir/network_identity.json")
soon_network_identity_pk=$(echo $soon_network_identity_key | solana-keygen pubkey -)

 echo -e "${YELLOW}🔧 Composing ${KEYS_JSON} with L1 EVM keys + Soon keys${NC}"
 jq -n \
       --arg identity_key "${soon_identity_key}" \
       --arg identity_pub "${soon_identity_pk}" \
       --arg upgrader_key "${soon_upgrader_key}" \
       --arg upgrader_pub "${soon_upgrader_pk}" \
       --arg faucet_key "${soon_faucet_key}" \
       --arg faucet_pub "${soon_faucet_pk}" \
       --arg network_key "${soon_network_identity_key}" \
       --arg network_pub "${soon_network_identity_pk}" \
       '{
         soon_identity: { private_key: $identity_key, public_key: $identity_pub },
         soon_upgrader: { private_key: $upgrader_key, public_key: $upgrader_pub },
         soon_faucet: { private_key: $faucet_key, public_key: $faucet_pub },
         soon_network_identity: { private_key: $network_key, public_key: $network_pub }
       }' > "${KEYS_JSON}"
echo -e "${GREEN}✅ keys prepared at ${KEYS_JSON}${NC}"

echo ""
echo -e "${YELLOW}🔧 Step 2: make genesis (Soon)${NC}"

NETWORK_NAME="${NETWORK_NAME}" \
SOON_DATA_PATH="${SOON_DATA_PATH}" \
L1_CHAIN_ID="${L1_CHAIN_ID}" \
L1_RPC_URL="${L1_RPC_URL}" \
ROLLUP_CONFIG_PATH="${E2E_DIR}" \
ARGS="--faucet-lamports 100000000000000" \
make -C "${SOON_ROOT}" genesis
echo -e "${GREEN}✅ genesis done${NC}"

echo ""
echo -e "${GREEN}🎉 L2 artifacts built successfully under: ${SOON_DATA_PATH}/.soon${NC}"
# Also place rollup.json in e2e root to match docker-compose mount
cp -f "${SOON_ROOT}/node/deployments/${NETWORK_NAME}.rollup.json" "${E2E_DIR}/rollup.json"
echo -e "${GREEN}✅ rollup.json placed at ${E2E_DIR}/rollup.json${NC}"
