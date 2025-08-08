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

# Get the directory where this script is located (e2e directory)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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
    if cp "$CONTRACTS_DIR/$file" "$SCRIPT_DIR/"; then
        echo -e "${GREEN}✅ Copied $file${NC}"
    else
        echo -e "${RED}❌ Failed to copy $file${NC}"
        exit 1
    fi
done

echo ""
echo -e "${GREEN}🎉 L1 node related files build completed successfully!${NC}"
echo ""

