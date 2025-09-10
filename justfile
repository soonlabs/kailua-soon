set fallback := true

# default recipe to display help information
default:
  @just --list

build +ARGS=" -F prove -F disable-dev-mode --locked":
  cargo build {{ARGS}}

build-fpvm +ARGS=" -F prove -F disable-dev-mode -F rebuild-fpvm --locked":
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

devnet-init-l1:
  ./e2e/init_l1_beacon.sh

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
  @docker volume rm e2e_l1_data e2e_l1_bn_data e2e_l1_vc_data 2>/dev/null || echo "⚠️  Docker volume e2e_l1_data not found or delete failed"
  rm -rf e2e/devnet
  rm -rf .localtestdata

devnet-verify:
  cd e2e && ./verify-contracts.sh

devnet-config target="debug" verbosity="" l1_rpc="http://127.0.0.1:8545" l2_rpc="http://127.0.0.1:8899" :
  ./target/{{target}}/kailua-cli config \
      --eth-rpc-url {{l1_rpc}} \
      --soon-node-url {{l2_rpc}} \
      --otlp-collector

devnet-upgrade timeout="3600" advantage="60" target="debug" verbosity="" l1_rpc="http://127.0.0.1:8545" l2_rpc="http://127.0.0.1:8899" vanguard="0x055A514d608c28F9F90eD2A6977f76e9DB08aFaD" deployer="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d" owner="0xc49af0e1e397697bd6a917a076d5cf4be42b91dfe307f7f3a07a07f9d50a3b89" guardian="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d":
  RISC0_DEV_MODE=1 ./target/{{target}}/kailua-cli fast-track \
      --eth-rpc-url {{l1_rpc}} \
      --soon-node-url {{l2_rpc}} \
      --starting-block-number 0 \
      --proposal-output-count 1 \
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

devnet-propose target="debug" verbosity="-vvv" da_proxy="http://127.0.0.1:8080/" l1_rpc="http://127.0.0.1:8545" l1_beacon_rpc="http://127.0.0.1:5052" l2_rpc="http://127.0.0.1:8899" data_dir=".localtestdata/propose" proposer="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d":
  ./target/{{target}}/kailua-cli propose \
      --eth-rpc-url {{l1_rpc}} \
      --beacon-rpc-url {{l1_beacon_rpc}} \
      --soon-node-url {{l2_rpc}} \
      --da-proxy-url {{da_proxy}} \
      --data-dir {{data_dir}} \
      --proposer-key {{proposer}} \
      {{verbosity}}

devnet-fault offset parent target="debug" proposer="0x5a2ca727946070dd1e37b79197681ee861a6b4e31b3a86d54396ead0b0bb03ac" verbosity="" da_proxy="http://127.0.0.1:8080/" l1_rpc="http://127.0.0.1:8545" l1_beacon_rpc="http://127.0.0.1:5052" l2_rpc="http://127.0.0.1:8899":
  ./target/{{target}}/kailua-cli test-fault \
      --eth-rpc-url {{l1_rpc}} \
      --beacon-rpc-url {{l1_beacon_rpc}} \
      --soon-node-url {{l2_rpc}} \
      --da-proxy-url {{da_proxy}} \
      --proposer-key {{proposer}} \
      --fault-offset {{offset}} \
      --fault-parent {{parent}} \
      {{verbosity}}

devnet-validate fastforward="100" target="debug" verbosity="" da_proxy="http://127.0.0.1:8080/" l1_rpc="http://127.0.0.1:8545" l1_beacon_rpc="http://127.0.0.1:5052" l2_rpc="http://127.0.0.1:8899" data_dir=".localtestdata/validate" validator="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d":
  ./target/{{target}}/kailua-cli validate \
      --fast-forward-target {{fastforward}} \
      --eth-rpc-url {{l1_rpc}} \
      --beacon-rpc-url {{l1_beacon_rpc}} \
      --soon-node-url {{l2_rpc}} \
      --da-proxy-url {{da_proxy}} \
      --kailua-host ./target/{{target}}/kailua-host \
      --data-dir {{data_dir}} \
      --validator-key {{validator}} \
      {{verbosity}}

devnet-validate-boundless fastforward="100" target="debug" verbosity="" da_proxy="http://127.0.0.1:8080/" l1_rpc="http://127.0.0.1:8545" l1_beacon_rpc="http://127.0.0.1:5052" l2_rpc="http://127.0.0.1:8899" data_dir=".localtestdata/validate" validator="0xe3cda83c742308a19c97c69089d33f848a1dc01467a912f514dd134953fd702d", set_verifier_address="0x1Ab08498CfF17b9723ED67143A050c8E8c2e3104", market_address="0x6B7ABa661041164b8dB98E30AE1454d2e9D5f14b":
    ./target/{{target}}/kailua-cli validate \
        --fast-forward-target {{fastforward}} \
        --eth-rpc-url {{l1_rpc}} \
        --beacon-rpc-url {{l1_beacon_rpc}} \
        --soon-node-url {{l2_rpc}} \
        --da-proxy-url {{da_proxy}} \
        --kailua-host ./target/{{target}}/kailua-host \
        --data-dir {{data_dir}} \
        --validator-key {{validator}} \
        --boundless-rpc-url ${BOUNDLESS_RPC_URL} \
        --boundless-wallet-key ${BOUNDLESS_WALLET_KEY} \
        --boundless-set-verifier-address {{set_verifier_address}} \
        --boundless-market-address {{market_address}} \
        --storage-provider pinata \
        --pinata-jwt ${BOUNDLESS_PINATA_JWT} \
        {{verbosity}}

devnet-prove block_number block_count="1" target="debug" verbosity="" data=".localtestdata": (prove block_number block_count "http://localhost:8545" "http://localhost:5052" "http://localhost:9545" "http://localhost:7545" data target verbosity)

bench l1_rpc l1_beacon_rpc l2_rpc da_proxy data start length range count target="release" verbosity="":
    ./target/{{target}}/kailua-cli benchmark \
          --eth-rpc-url {{l1_rpc}} \
          --beacon-rpc-url {{l1_beacon_rpc}} \
          --soon-node-url {{l2_rpc}} \
          --da-proxy-url {{da_proxy}} \
          --data-dir {{data}} \
          --bench-start {{start}} \
          --bench-length {{length}} \
          --bench-range {{range}} \
          --bench-count {{count}} \
          {{verbosity}}

# Run the client program natively with the host program attached.
prove block_number block_count l1_rpc l1_beacon_rpc l2_rpc da_proxy data target="release" seq_window="50" verbosity="":
  #!/usr/bin/env bash

  L1_NODE_ADDRESS="{{l1_rpc}}"
  L1_BEACON_ADDRESS="{{l1_beacon_rpc}}"
  SOON_NODE_URL="{{l2_rpc}}"
  DA_PROXY_URL="{{da_proxy}}"

  L2_BLOCK_NUMBER={{block_number}}
  CLAIMED_L2_BLOCK_NUMBER=$((L2_BLOCK_NUMBER + {{block_count}}))

  # Get output root for block
  echo "Fetching data for block #$CLAIMED_L2_BLOCK_NUMBER..."
  CLAIMED_L2_OUTPUT_ROOT=$(cast rpc --rpc-url $SOON_NODE_URL "outputAtBlock" $(cast 2h $CLAIMED_L2_BLOCK_NUMBER) | jq -r .outputRoot)
  # Get the info for the origin l1 block
  L1_ORIGIN_NUM=$(cast rpc --rpc-url $SOON_NODE_URL "outputAtBlock" $(cast 2h $CLAIMED_L2_BLOCK_NUMBER) | jq -r .blockRef.l1origin.number)
  L1_HEAD=$(cast block --rpc-url $L1_NODE_ADDRESS $((L1_ORIGIN_NUM + {{seq_window}})) --json | jq -r .hash)

  # Get the info for the parent l2 block
  echo "Fetching data for parent of block #$L2_BLOCK_NUMBER..."
  AGREED_L2_OUTPUT_ROOT=$(cast rpc --rpc-url $SOON_NODE_URL "outputAtBlock" $(cast 2h $L2_BLOCK_NUMBER) | jq -r .outputRoot)

  echo "Running host program with zk client program..."
  ./target/{{target}}/kailua-host {{verbosity}} \
    --l2_node_address $SOON_NODE_URL \
    --l1-head $L1_HEAD \
    --agreed-l2-block-number $L2_BLOCK_NUMBER \
    --agreed-l2-output-root $AGREED_L2_OUTPUT_ROOT \
    --claimed-l2-output-root $CLAIMED_L2_OUTPUT_ROOT \
    --claimed-l2-block-number $CLAIMED_L2_BLOCK_NUMBER \
    --l2-chain-id 0 \
    --l1-node-address $L1_NODE_ADDRESS \
    --l1-beacon-address $L1_BEACON_ADDRESS \
    --da-proxy-url $DA_PROXY_URL \
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

prove-offline block_number l1_head l2_block_number l2_claim l2_output_root l2_chain_id data target="release" verbosity="":
  echo "Running host program with zk client program..."
  NUM_CONCURRENT_PREFLIGHTS=0 ./target/{{target}}/kailua-host {{verbosity}} \
    --l1-head {{l1_head}} \
    --agreed-l2-block-number {{l2_block_number}} \
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
