#!/bin/bash

# Script to build L1 node related files for e2e testing
set -e

# Color definitions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default configuration
CONTRACTS_DIR="${CONTRACTS_DIR:-../soon/contracts/dump}"

echo -e "${BLUE}🚀 Building L1 node related files for e2e testing${NC}"
echo "=================================================="

# Step 0: Check Go version
echo -e "${YELLOW}🔧 Step 0: Checking Go version...${NC}"

# Get the directory where this script is located (e2e directory)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Get required Go version from version.json
REQUIRED_GO_VERSION=$(jq -r .go < "$SCRIPT_DIR/version.json")
echo -e "${BLUE}📋 Required Go version: $REQUIRED_GO_VERSION${NC}"

# Check if Go is installed
if ! command -v go &> /dev/null; then
    echo -e "${RED}❌ Go is not installed${NC}"
    echo "Please install Go version $REQUIRED_GO_VERSION first"
    exit 1
fi

# Get current Go version
CURRENT_GO_VERSION=$(go version | awk '{print $3}' | sed 's/go//')
echo -e "${BLUE}📋 Current Go version: $CURRENT_GO_VERSION${NC}"

# Compare versions
if [ "$CURRENT_GO_VERSION" -lt "$REQUIRED_GO_VERSION" ]; then
    echo -e "${RED}❌ Go version mismatch${NC}"
    echo "Current: $CURRENT_GO_VERSION"
    echo "Required: $REQUIRED_GO_VERSION"
    echo "Please install Go version $REQUIRED_GO_VERSION"
    exit 1
else
    echo -e "${GREEN}✅ Go version matches requirement${NC}"
fi

echo ""

# Check if contracts directory exists
if [ ! -d "$CONTRACTS_DIR" ]; then
    echo -e "${RED}❌ Contracts directory does not exist: $CONTRACTS_DIR${NC}"
    echo "Please check the CONTRACTS_DIR path or set it via environment variable:"
    echo "  CONTRACTS_DIR=/path/to/contracts/dump $0"
    exit 1
fi

# Check if Makefile exists in contracts directory
if [ ! -f "$CONTRACTS_DIR/Makefile" ]; then
    echo -e "${RED}❌ Makefile not found in: $CONTRACTS_DIR${NC}"
    echo "Please ensure the contracts directory contains a Makefile"
    exit 1
fi

echo -e "${YELLOW}📁 Using contracts directory: $CONTRACTS_DIR${NC}"
echo ""

# Step 1: Generate keys
echo -e "${YELLOW}🔧 Step 1: Generate keys...${NC}"
if make -C "$CONTRACTS_DIR" keys; then
    echo -e "${GREEN}✅ Keys generated successfully${NC}"
else
    echo -e "${RED}❌ Failed to generate keys${NC}"
    exit 1
fi
echo ""

# Step 2: Dump state for genesis
echo -e "${YELLOW}🔧 Step 2: Dump state for genesis...${NC}"
if make -C "$CONTRACTS_DIR" allocs; then
    echo -e "${GREEN}✅ State dumped successfully${NC}"
else
    echo -e "${RED}❌ Failed to dump state${NC}"
    exit 1
fi
echo ""

# Step 3: Generate genesis file
echo -e "${YELLOW}🔧 Step 3: Generate genesis file...${NC}"
if make -C "$CONTRACTS_DIR" genesis; then
    echo -e "${GREEN}✅ Genesis file generated successfully${NC}"
else
    echo -e "${RED}❌ Failed to generate genesis file${NC}"
    exit 1
fi
echo ""

# Step 4: Copy files to e2e directory
echo -e "${YELLOW}🔧 Step 4: Copy files to e2e directory...${NC}"

# Check if devnet directory exists, create if it doesn't
DEVNET_DIR="$SCRIPT_DIR/devnet"
if [ ! -d "$DEVNET_DIR" ]; then
    echo -e "${YELLOW}📁 Creating devnet directory...${NC}"
    if mkdir -p "$DEVNET_DIR"; then
        echo -e "${GREEN}✅ Created devnet directory: $DEVNET_DIR${NC}"
    else
        echo -e "${RED}❌ Failed to create devnet directory${NC}"
        exit 1
    fi
else
    echo -e "${GREEN}✅ Devnet directory already exists: $DEVNET_DIR${NC}"
fi

# Files to copy
FILES_TO_COPY=(
    "genesis.json"
    "addresses.json" 
    "genesis-keys.json"
)

# Check if source files exist
MISSING_FILES=()
for file in "${FILES_TO_COPY[@]}"; do
    if [ ! -f "$CONTRACTS_DIR/$file" ]; then
        MISSING_FILES+=("$file")
    fi
done

if [ ${#MISSING_FILES[@]} -gt 0 ]; then
    echo -e "${RED}❌ Missing required files in $CONTRACTS_DIR:${NC}"
    for file in "${MISSING_FILES[@]}"; do
        echo "  - $file"
    done
    exit 1
fi

# Copy files
for file in "${FILES_TO_COPY[@]}"; do
    if cp "$CONTRACTS_DIR/$file" "$DEVNET_DIR/"; then
        echo -e "${GREEN}✅ Copied $file${NC}"
    else
        echo -e "${RED}❌ Failed to copy $file${NC}"
        exit 1
    fi
done

echo ""
echo -e "${GREEN}🎉 L1 node related files build completed successfully!${NC}"
echo ""

# Step 5: Generate .env from template (if exists) and update required values
echo -e "${YELLOW}🔧 Step 5: Generate .env for docker-compose${NC}"

ENV_EXAMPLE_PATH="$SCRIPT_DIR/.env.example"
ENV_PATH="$SCRIPT_DIR/.env"

# Extract values from generated files
PROPOSER_ADMIN_SECRET=$(jq -r '.gs_proposer.private_key' "$SCRIPT_DIR/devnet/genesis-keys.json")
BATCHER_ADMIN_SECRET=$(jq -r '.gs_batcher.private_key' "$SCRIPT_DIR/devnet/genesis-keys.json")
L2OO_ADDRESS=$(jq -r '.L2OutputOracleProxy' "$SCRIPT_DIR/devnet/addresses.json")
GAME_FACTORY_ADDRESS=$(jq -r '.DisputeGameFactoryProxy' "$SCRIPT_DIR/devnet/addresses.json")

if [ -f "$ENV_EXAMPLE_PATH" ]; then
  cp -f "$ENV_EXAMPLE_PATH" "$ENV_PATH"
else
  # Create a minimal .env if no template exists
  cat > "$ENV_PATH" <<EOF
# Autogenerated .env (no .env.example found)
EOF
fi

update_env_var() {
  local key="$1"
  local value="$2"
  if grep -qE "^${key}=" "$ENV_PATH"; then
    sed -i -E "s|^${key}=.*$|${key}=${value}|" "$ENV_PATH"
  else
    echo "${key}=${value}" >> "$ENV_PATH"
  fi
}

update_env_var PROPOSER_ADMIN_SECRET "$PROPOSER_ADMIN_SECRET"
update_env_var BATCHER_ADMIN_SECRET "$BATCHER_ADMIN_SECRET"
update_env_var L2OO_ADDRESS "$L2OO_ADDRESS"
update_env_var GAME_FACTORY_ADDRESS "$GAME_FACTORY_ADDRESS"

echo -e "${GREEN}✅ .env generated/updated at ${ENV_PATH}${NC}"

# Step 6: Build beacon chain genesis
echo ""
echo -e "${YELLOW}🔧 Step 6: Building beacon chain genesis...${NC}"

echo -e "${BLUE}🔍 Checking if eth2-testnet-genesis is installed...${NC}"

# Check if eth2-testnet-genesis is available
if ! command -v eth2-testnet-genesis &> /dev/null; then
    echo -e "${YELLOW}❌ eth2-testnet-genesis not found. Installing...${NC}"
    
    # Get the version from version.json
    ETH2_TESTNET_GENESIS_VERSION=$(jq -r .eth2_testnet_genesis < $SCRIPT_DIR/version.json)
    echo -e "${BLUE}📦 Installing eth2-testnet-genesis version: $ETH2_TESTNET_GENESIS_VERSION${NC}"
    
    # Install eth2-testnet-genesis
    if go install -v github.com/protolambda/eth2-testnet-genesis@$ETH2_TESTNET_GENESIS_VERSION; then
        echo -e "${GREEN}✅ eth2-testnet-genesis installed successfully${NC}"
    else
        echo -e "${RED}❌ Failed to install eth2-testnet-genesis${NC}"
        exit 1
    fi
else
    echo -e "${GREEN}✅ eth2-testnet-genesis is already installed${NC}"
fi

echo -e "${BLUE}🚀 Generating beacon-chain genesis...${NC}"

echo "eth2-testnet-genesis path: $(which eth2-testnet-genesis)"

if eth2-testnet-genesis deneb \
  --config=$SCRIPT_DIR/beacon-data/config.yaml \
  --preset-phase0=minimal \
  --preset-altair=minimal \
  --preset-bellatrix=minimal \
  --preset-capella=minimal \
  --preset-deneb=minimal \
  --eth1-config=$DEVNET_DIR/genesis.json \
  --state-output=$DEVNET_DIR/genesis-l1.ssz \
  --tranches-dir=$DEVNET_DIR/tranches \
  --mnemonics=$SCRIPT_DIR/mnemonics.yaml \
  --eth1-withdrawal-address=0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --eth1-match-genesis-time; then
    echo -e "${GREEN}✅ Beacon genesis generation completed successfully${NC}"
else
    echo -e "${RED}❌ Failed to generate beacon genesis${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}🎉 All L1 and beacon chain files build completed successfully!${NC}"

