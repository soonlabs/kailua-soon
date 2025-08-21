#!/bin/bash

# Script to initialize L1 beacon chain data for e2e testing
set -e

# Color definitions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Get the directory where this script is located (e2e directory)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BEACON_DATA_DIR="$SCRIPT_DIR/beacon-data"

echo -e "${BLUE}🚀 Initializing L1 beacon chain data for e2e testing${NC}"
echo "======================================================="

# Step 1: Check if eth2-val-tools is installed
echo -e "${YELLOW}🔧 Step 1: Checking eth2-val-tools...${NC}"

if ! command -v eth2-val-tools &> /dev/null; then
    echo -e "${YELLOW}❌ eth2-val-tools not found. Installing...${NC}"
    
    echo -e "${BLUE}📦 Installing eth2-val-tools latest version${NC}"
    
    # Install eth2-val-tools
    if go install github.com/protolambda/eth2-val-tools@latest; then
        echo -e "${GREEN}✅ eth2-val-tools installed successfully${NC}"
    else
        echo -e "${RED}❌ Failed to install eth2-val-tools${NC}"
        exit 1
    fi
else
    echo -e "${GREEN}✅ eth2-val-tools is already installed${NC}"
fi

echo ""

# Step 2: Check if beacon-data directory exists
echo -e "${YELLOW}🔧 Step 2: Checking beacon-data directory...${NC}"

if [ ! -d "$BEACON_DATA_DIR" ]; then
    echo -e "${RED}❌ beacon-data directory not found: $BEACON_DATA_DIR${NC}"
    exit 1
fi

echo -e "${GREEN}✅ beacon-data directory found: $BEACON_DATA_DIR${NC}"

# Check if Makefile exists
if [ ! -f "$BEACON_DATA_DIR/Makefile" ]; then
    echo -e "${RED}❌ Makefile not found in beacon-data directory${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Makefile found in beacon-data directory${NC}"
echo ""

# Step 3: Clean existing data if it exists
echo -e "${YELLOW}🔧 Step 3: Cleaning existing beacon data...${NC}"

BEACON_DATA_DATA_DIR="$BEACON_DATA_DIR/data"
if [ -d "$BEACON_DATA_DATA_DIR" ]; then
    echo -e "${YELLOW}🗑️  Removing existing data directory: $BEACON_DATA_DATA_DIR${NC}"
    if rm -rf "$BEACON_DATA_DATA_DIR"; then
        echo -e "${GREEN}✅ Existing data directory removed${NC}"
    else
        echo -e "${RED}❌ Failed to remove existing data directory${NC}"
        exit 1
    fi
else
    echo -e "${BLUE}ℹ️  No existing data directory found${NC}"
fi

echo ""

# Step 4: Generate beacon data
echo -e "${YELLOW}🔧 Step 4: Generating beacon chain data...${NC}"

echo -e "${BLUE}🚀 Running make data in beacon-data directory...${NC}"

# Change to beacon-data directory and run make data
if make -C "$BEACON_DATA_DIR" data; then
    echo -e "${GREEN}✅ Beacon chain data generated successfully${NC}"
else
    echo -e "${RED}❌ Failed to generate beacon chain data${NC}"
    exit 1
fi

echo ""
