set fallback := true

# default recipe to display help information
default:
  @just --list

build +ARGS="--release -F prove -F disable-dev-mode --locked":
  cargo build {{ARGS}}

build-fpvm +ARGS="--release -F prove -F disable-dev-mode -F rebuild-fpvm --locked":
  cargo build {{ARGS}}

fmt:
  cargo fmt --all

  cargo fmt --all --manifest-path build/risczero/fpvm/Cargo.toml

clippy:
  RISC0_SKIP_BUILD=true cargo clippy --locked --workspace --all --all-targets -- -D warnings

  cargo clippy --manifest-path build/risczero/fpvm/Cargo.toml --locked --workspace --all --all-targets -- -D warnings

coverage:
  cargo +nightly llvm-cov -p kailua-common --branch

coverage-open:
  cargo +nightly llvm-cov -p kailua-common --branch --open

devnet-build-l1 CONTRACTS_DIR="../soon/contracts/dump" +ARGS="-F devnet -F prove":
  CONTRACTS_DIR={{CONTRACTS_DIR}} ./e2e/build_l1.sh

devnet-build-l2 soon_root="../soon" network_name="ethereum.testnet" l1_chain_id="11155111" l1_rpc_url="http://localhost:8545" +ARGS="-F devnet -F prove":
  SOON_ROOT={{soon_root}} NETWORK_NAME={{network_name}} L1_CHAIN_ID={{l1_chain_id}} L1_RPC_URL={{l1_rpc_url}} ./e2e/build_l2.sh

devnet-build +ARGS="-F devnet -F prove": (build ARGS)

devnet-build-fpvm +ARGS="-F devnet -F prove -F rebuild-fpvm": (build ARGS)

devnet-up-l1:
  cd e2e && docker compose up -d l1 l1-bn l1-vc

devnet-up:
  cd e2e && docker compose up -d

devnet-logs:
  cd e2e && docker compose logs -f

devnet-log CONTAINER:
  cd e2e && docker compose logs -f {{CONTAINER}}

devnet-down:
  cd e2e && docker compose down

devnet-clean: devnet-down
  @docker volume rm e2e_l1_data 2>/dev/null || echo "⚠️  Docker volume e2e_l1_data not found or delete failed"
  rm -rf e2e/devnet

devnet-verify:
  cd e2e && ./verify-contracts.sh

devnet-config target="debug" verbosity="" l1_rpc="http://127.0.0.1:8545" l2_rpc="http://127.0.0.1:9545" rollup_node_rpc="http://127.0.0.1:7545":
  ./target/{{target}}/kailua-cli config \
      --eth-rpc-url {{l1_rpc}} \
      --op-geth-url {{l2_rpc}} \
      --op-node-url {{rollup_node_rpc}} \
      --otlp-collector

devnet-upgrade timeout="3600" advantage="60" target="debug" verbosity="" l1_rpc="http://127.0.0.1:8545" l2_rpc="http://127.0.0.1:8899" vanguard="0x055A514d608c28F9F90eD2A6977f76e9DB08aFaD" deployer="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d" owner="0xc49af0e1e397697bd6a917a076d5cf4be42b91dfe307f7f3a07a07f9d50a3b89" guardian="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d":
  RISC0_DEV_MODE=1 ./target/{{target}}/kailua-cli fast-track \
      --eth-rpc-url {{l1_rpc}} \
      --soon-node-url {{l2_rpc}} \
      --starting-block-number 0 \
      --proposal-output-count 20 \
      --output-block-span 50 \
      --challenge-timeout {{timeout}} \
      --collateral-amount 1 \
      --deployer-key {{deployer}} \
      --owner-key {{owner}} \
      --guardian-key {{guardian}} \
      --vanguard-address {{vanguard}} \
      --respect-kailua-proposals \
      {{verbosity}}

devnet-reset: devnet-down devnet-clean devnet-up

devnet-propose target="debug" verbosity="" l1_rpc="http://127.0.0.1:8545" l1_beacon_rpc="http://127.0.0.1:5052" l2_rpc="http://127.0.0.1:8899" data_dir=".localtestdata/propose" proposer="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d":
  ./target/{{target}}/kailua-cli propose \
      --eth-rpc-url {{l1_rpc}} \
      --beacon-rpc-url {{l1_beacon_rpc}} \
      --soon-node-url {{l2_rpc}} \
      --data-dir {{data_dir}} \
      --proposer-key {{proposer}} \
      {{verbosity}}

devnet-fault offset parent target="debug" proposer="0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a" verbosity="" l1_rpc="http://127.0.0.1:8545" l1_beacon_rpc="http://127.0.0.1:5052" l2_rpc="http://127.0.0.1:9545" rollup_node_rpc="http://127.0.0.1:7545":
  ./target/{{target}}/kailua-cli test-fault \
      --eth-rpc-url {{l1_rpc}} \
      --beacon-rpc-url {{l1_beacon_rpc}} \
      --op-geth-url {{l2_rpc}} \
      --op-node-url {{rollup_node_rpc}} \
      --proposer-key {{proposer}} \
      --fault-offset {{offset}} \
      --fault-parent {{parent}} \
      {{verbosity}}

devnet-validate fastforward="0" target="debug" verbosity="" l1_rpc="http://127.0.0.1:8545" l1_beacon_rpc="http://127.0.0.1:5052" l2_rpc="http://127.0.0.1:9545" rollup_node_rpc="http://127.0.0.1:7545" data_dir=".localtestdata/validate" validator="0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e":
  ./target/{{target}}/kailua-cli validate \
      --fast-forward-target {{fastforward}} \
      --eth-rpc-url {{l1_rpc}} \
      --beacon-rpc-url {{l1_beacon_rpc}} \
      --op-geth-url {{l2_rpc}} \
      --op-node-url {{rollup_node_rpc}} \
      --kailua-host ./target/{{target}}/kailua-host \
      --data-dir {{data_dir}} \
      --validator-key {{validator}} \
      {{verbosity}}

devnet-prove block_number block_count="1" target="debug" verbosity="" data=".localtestdata": (prove block_number block_count "http://localhost:8545" "http://localhost:5052" "http://localhost:9545" "http://localhost:7545" data target verbosity)

bench l1_rpc l1_beacon_rpc l2_rpc rollup_node_rpc data start length range count target="release" verbosity="":
    ./target/{{target}}/kailua-cli benchmark \
          --eth-rpc-url {{l1_rpc}} \
          --beacon-rpc-url {{l1_beacon_rpc}} \
          --op-geth-url {{l2_rpc}} \
          --op-node-url {{rollup_node_rpc}} \
          --data-dir {{data}} \
          --bench-start {{start}} \
          --bench-length {{length}} \
          --bench-range {{range}} \
          --bench-count {{count}} \
          {{verbosity}}

# Run the client program natively with the host program attached.
prove block_number block_count l1_rpc l1_beacon_rpc l2_rpc rollup_node_rpc data target="release" seq_window="50" verbosity="":
  #!/usr/bin/env bash

  L1_NODE_ADDRESS="{{l1_rpc}}"
  L1_BEACON_ADDRESS="{{l1_beacon_rpc}}"
  L2_NODE_ADDRESS="{{l2_rpc}}"
  OP_NODE_ADDRESS="{{rollup_node_rpc}}"

  L2_BLOCK_NUMBER={{block_number}}
  CLAIMED_L2_BLOCK_NUMBER=$((L2_BLOCK_NUMBER + {{block_count}} - 1))

  # Query the chain id
  echo "Fetching chain id"
  L2_CHAIN_ID=$(cast chain-id --rpc-url $L2_NODE_ADDRESS)

  # Get output root for block
  echo "Fetching data for block #$CLAIMED_L2_BLOCK_NUMBER..."
  CLAIMED_L2_OUTPUT_ROOT=$(cast rpc --rpc-url $OP_NODE_ADDRESS "optimism_outputAtBlock" $(cast 2h $CLAIMED_L2_BLOCK_NUMBER) | jq -r .outputRoot)
  # Get the info for the origin l1 block
  L1_ORIGIN_NUM=$(cast rpc --rpc-url $OP_NODE_ADDRESS "optimism_outputAtBlock" $(cast 2h $CLAIMED_L2_BLOCK_NUMBER) | jq -r .blockRef.l1origin.number)
  L1_HEAD=$(cast block --rpc-url $L1_NODE_ADDRESS $((L1_ORIGIN_NUM + {{seq_window}})) --json | jq -r .hash)

  # Get the info for the parent l2 block
  echo "Fetching data for parent of block #$L2_BLOCK_NUMBER..."
  AGREED_L2_OUTPUT_ROOT=$(cast rpc --rpc-url $OP_NODE_ADDRESS "optimism_outputAtBlock" $(cast 2h $((L2_BLOCK_NUMBER - 1))) | jq -r .outputRoot)
  AGREED_L2_HEAD=$(cast block --rpc-url $L2_NODE_ADDRESS $((L2_BLOCK_NUMBER - 1)) --json | jq -r .hash)

  echo "Running host program with zk client program..."
  ./target/{{target}}/kailua-host {{verbosity}} \
    --op-node-address $OP_NODE_ADDRESS \
    --l1-head $L1_HEAD \
    --agreed-l2-head-hash $AGREED_L2_HEAD \
    --agreed-l2-output-root $AGREED_L2_OUTPUT_ROOT \
    --claimed-l2-output-root $CLAIMED_L2_OUTPUT_ROOT \
    --claimed-l2-block-number $CLAIMED_L2_BLOCK_NUMBER \
    --l2-chain-id $L2_CHAIN_ID \
    --l1-node-address $L1_NODE_ADDRESS \
    --l1-beacon-address $L1_BEACON_ADDRESS \
    --l2-node-address $L2_NODE_ADDRESS \
    --data-dir {{data}} \
    --native

# Show the input args for proving
query block_number l1_rpc l1_beacon_rpc l2_rpc rollup_node_rpc seq_window="50":
  #!/usr/bin/env bash

  L1_NODE_ADDRESS="{{l1_rpc}}"
  L1_BEACON_ADDRESS="{{l1_beacon_rpc}}"
  L2_NODE_ADDRESS="{{l2_rpc}}"
  OP_NODE_ADDRESS="{{rollup_node_rpc}}"

  L2_BLOCK_NUMBER={{block_number}}

  echo "Fetching data for block #$L2_BLOCK_NUMBER..."
  L1_ORIGIN_NUM=$(cast rpc --rpc-url $OP_NODE_ADDRESS "optimism_outputAtBlock" $(cast 2h $((L2_BLOCK_NUMBER - 1))) | jq -r .blockRef.l1origin.number)

  echo $L1_ORIGIN_NUM
  # L1 head
  cast block --rpc-url $L1_NODE_ADDRESS $((L1_ORIGIN_NUM + {{seq_window}})) --json | jq -r .hash
  # L2 hash
  cast block --rpc-url $L2_NODE_ADDRESS $((L2_BLOCK_NUMBER - 1)) --json | jq -r .hash
  # L2 Claim
  cast rpc --rpc-url $OP_NODE_ADDRESS "optimism_outputAtBlock" $(cast 2h $L2_BLOCK_NUMBER) | jq -r .outputRoot
  # L2 agreed output root
  cast rpc --rpc-url $OP_NODE_ADDRESS "optimism_outputAtBlock" $(cast 2h $((L2_BLOCK_NUMBER - 1))) | jq -r .outputRoot
  # L2 chain id
  cast chain-id --rpc-url $L2_NODE_ADDRESS

prove-offline block_number l1_head l2_hash l2_claim l2_output_root l2_chain_id data target="release" verbosity="":
  echo "Running host program with zk client program..."
  NUM_CONCURRENT_PREFLIGHTS=0 ./target/{{target}}/kailua-host {{verbosity}} \
    --l1-head {{l1_head}} \
    --agreed-l2-head-hash {{l2_hash}} \
    --claimed-l2-output-root {{l2_claim}} \
    --agreed-l2-output-root {{l2_output_root}} \
    --claimed-l2-block-number {{block_number}} \
    --l2-chain-id {{l2_chain_id}} \
    --data-dir {{data}} \
    --native

test verbosity="":
    echo "Running cargo tests"
    RISC0_DEV_MODE=1 cargo test -F devnet

test-offline target="release" verbosity="": (prove-offline "16491249" "0x33a3e5721faa4dc6f25e75000d9810fd6c41320868f3befcc0c261a71da398e1" "0x09b298a83baf4c2e3c6a2e355bb09e27e3fdca435080e8754f8749233d7333b2" "0x82da7204148ba4d8d59e587b6b3fdde5561dc31d9e726220f7974bf9f2158d75" "0xa548f22e1aa590de7ed271e3eab5b66c6c3db9b8cb0e3f91618516ea9ececde4" "11155420" "./testdata/16491249" target verbosity)

cleanup:
    echo "Cleanup: Removing any .fake receipt files in directory."
    rm ./*.fake
