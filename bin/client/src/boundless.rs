// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::borrow::Cow;
use crate::proving::{ProvingArgs, ProvingError};
use alloy::transports::http::reqwest::Url;
use alloy_primitives::{Address, U256};
use anyhow::{anyhow, bail, Context};
use boundless_market::alloy::providers::Provider;
use boundless_market::alloy::signers::local::PrivateKeySigner;
use boundless_market::alloy::eips::BlockNumberOrTag;
use boundless_market::client::Client;
use boundless_market::contracts::{Predicate, RequestId, RequestStatus, Requirements};
use boundless_market::storage::{StorageProviderConfig, StorageProviderType};
use boundless_market::{Deployment, GuestEnvBuilder};
use boundless_market::request_builder::OfferParams;
use clap::Parser;
use kailua_build::{KAILUA_FPVM_ELF, KAILUA_FPVM_ID};
use kailua_common::journal::ProofJournal;
use risc0_ethereum_contracts::selector::Selector;
use risc0_zkvm::sha::Digestible;
use risc0_zkvm::{default_executor, ExecutorEnv, Journal, Receipt};
use std::time::Duration;
use tracing::info;
use tracing::log::warn;

#[derive(Parser, Clone, Debug)]
pub struct BoundlessArgs {
    /// Market provider for proof requests
    #[clap(flatten)]
    pub market: Option<MarketProviderConfig>,
    /// Storage provider for elf and input
    #[clap(flatten)]
    pub storage: StorageProviderConfig,
}

#[derive(Parser, Debug, Clone)]
#[group(requires_all = ["boundless_rpc_url", "boundless_wallet_key", "boundless_set_verifier_address", "boundless_market_address"])]
pub struct MarketProviderConfig {
    /// URL of the Ethereum RPC endpoint.
    #[clap(long, env)]
    #[arg(required = false)]
    pub boundless_rpc_url: Url,
    /// Private key used to interact with the EvenNumber contract.
    #[clap(long, env)]
    #[arg(required = false)]
    pub boundless_wallet_key: PrivateKeySigner,

    /// EIP-155 chain ID of the network hosting Boundless.
    ///
    /// This parameter takes precedent over all other deployment arguments if set to a known value
    #[clap(long, env, required = false)]
    pub boundless_chain_id: Option<u64>,
    /// Address of the [BoundlessMarket] contract.
    ///
    /// [BoundlessMarket]: crate::contracts::IBoundlessMarket
    #[clap(long, env, required = false)]
    pub boundless_market_address: Option<Address>,
    /// Address of the [RiscZeroVerifierRouter] contract.
    ///
    /// The verifier router implements [IRiscZeroVerifier]. Each network has a canonical router,
    /// that is deployed by the core team. You can additionally deploy and manage your own verifier
    /// instead. See the [Boundless docs for more details].
    ///
    /// [RiscZeroVerifierRouter]: https://github.com/risc0/risc0-ethereum/blob/main/contracts/src/RiscZeroVerifierRouter.sol
    /// [IRiscZeroVerifier]: https://github.com/risc0/risc0-ethereum/blob/main/contracts/src/IRiscZeroVerifier.sol
    /// [Boundless docs for more details]: https://docs.beboundless.xyz/developers/smart-contracts/verifier-contracts
    #[clap(
        long,
        env = "VERIFIER_ADDRESS",
        required = false,
        long_help = "Address of the RiscZeroVerifierRouter contract"
    )]
    pub boundless_verifier_router_address: Option<Address>,
    /// Address of the [RiscZeroSetVerifier] contract.
    ///
    /// [RiscZeroSetVerifier]: https://github.com/risc0/risc0-ethereum/blob/main/contracts/src/RiscZeroSetVerifier.sol
    #[clap(long, env, required = false)]
    pub boundless_set_verifier_address: Option<Address>,
    /// Address of the stake token contract. The staking token is an ERC-20.
    #[clap(long, env, required = false)]
    pub boundless_stake_token_address: Option<Address>,
    /// URL for the offchain [order stream service].
    ///
    /// [order stream service]: crate::order_stream_client
    #[clap(
        long,
        env,
        required = false,
        long_help = "URL for the offchain order stream service"
    )]
    pub boundless_order_stream_url: Option<Cow<'static, str>>,

    /// Number of transactions to lookback at
    #[clap(long, env)]
    #[arg(required = false, default_value_t = 5)]
    pub boundless_lookback: u32,

    /// Starting price (wei) per cycle of the proving order
    #[clap(long, env, required = false, default_value = "0")]
    pub boundless_cycle_min_wei: U256,
    /// Maximum price (wei) per cycle of the proving order
    #[clap(long, env, required = false, default_value = "200000")]
    pub boundless_cycle_max_wei: U256,
    /// Stake (USDC) per million cycles of the proving order
    #[clap(long, env, required = false, default_value = "1000")]
    pub boundless_mega_mcycle_stake: U256,
    /// Multiplier for delay before order price starts ramping up.
    #[clap(long, env, required = false, default_value_t = 2.0)]
    pub boundless_order_bid_delay_factor: f64,
    /// Multiplier for order price to ramp up from min to max.
    #[clap(long, env, required = false, default_value_t = 3.0)]
    pub boundless_order_ramp_up_factor: f64,
    /// Multiplier for order fulfillment timeout (seconds/segment) after locking
    #[clap(long, env, required = false, default_value_t = 5.0)]
    pub boundless_order_lock_timeout_factor: f64,
    /// Multiplier for order expiry timeout (seconds/segment) after lock timeout
    #[clap(long, env, required = false, default_value_t = 3.0)]
    pub boundless_order_expiry_factor: f64,
    /// Time in seconds between attempts to check order status
    #[clap(long, env, required = false, default_value_t = 12)]
    pub boundless_order_check_interval: u64,
}

impl MarketProviderConfig {
    pub fn to_arg_vec(
        &self,
        storage_provider_config: &StorageProviderConfig,
    ) -> Vec<String> {
        let mut proving_args = Vec::new();
        proving_args.extend(vec![
            String::from("--boundless-rpc-url"),
            self.boundless_rpc_url.to_string(),
            String::from("--boundless-wallet-key"),
            self.boundless_wallet_key.to_bytes().to_string(),
            String::from("--boundless-lookback"),
            self.boundless_lookback.to_string(),
            String::from("--boundless-cycle-min-wei"),
            self.boundless_cycle_min_wei.to_string(),
            String::from("--boundless-cycle-max-wei"),
            self.boundless_cycle_max_wei.to_string(),
            String::from("--boundless-mega-mcycle-stake"),
            self.boundless_mega_mcycle_stake.to_string(),
            String::from("--boundless-order-bid-delay-factor"),
            self.boundless_order_bid_delay_factor.to_string(),
            String::from("--boundless-order-ramp-up-factor"),
            self.boundless_order_ramp_up_factor.to_string(),
            String::from("--boundless-order-lock-timeout-factor"),
            self.boundless_order_lock_timeout_factor.to_string(),
            String::from("--boundless-order-expiry-factor"),
            self.boundless_order_expiry_factor.to_string(),
            String::from("--boundless-order-check-interval"),
            self.boundless_order_check_interval.to_string(),
        ]);
        if let Some(chain_id) = &self.boundless_chain_id {
            proving_args.extend(vec![
                String::from("--boundless-chain-id"),
                chain_id.to_string(),
            ]);
        }
        if let Some(address) = &self.boundless_market_address {
            proving_args.extend(vec![
                String::from("--boundless-market-address"),
                address.to_string(),
            ]);
        }
        if let Some(address) = &self.boundless_verifier_router_address {
            proving_args.extend(vec![
                String::from("--boundless-verifier-router-address"),
                address.to_string(),
            ]);
        }
        if let Some(address) = &self.boundless_set_verifier_address {
            proving_args.extend(vec![
                String::from("--boundless-set-verifier-address"),
                address.to_string(),
            ]);
        }
        if let Some(address) = &self.boundless_stake_token_address {
            proving_args.extend(vec![
                String::from("--boundless-stake-token-address"),
                address.to_string(),
            ]);
        }
        if let Some(url) = &self.boundless_order_stream_url {
            proving_args.extend(vec![
                String::from("--boundless-order-stream-url"),
                url.to_string(),
            ]);
        }

        match &storage_provider_config.storage_provider {
            StorageProviderType::S3 => {
                proving_args.extend(vec![
                    String::from("--storage-provider"),
                    String::from("s3"),
                    String::from("--s3-access-key"),
                    storage_provider_config.s3_access_key.clone().unwrap(),
                    String::from("--s3-secret-key"),
                    storage_provider_config.s3_secret_key.clone().unwrap(),
                    String::from("--s3-bucket"),
                    storage_provider_config.s3_bucket.clone().unwrap(),
                    String::from("--s3-url"),
                    storage_provider_config.s3_url.clone().unwrap(),
                    String::from("--aws-region"),
                    storage_provider_config.aws_region.clone().unwrap(),
                ]);
            }
            StorageProviderType::Pinata => {
                proving_args.extend(vec![
                    String::from("--storage-provider"),
                    String::from("pinata"),
                    String::from("--pinata-jwt"),
                    storage_provider_config.pinata_jwt.clone().unwrap(),
                ]);
                if let Some(pinata_api_url) = &storage_provider_config.pinata_api_url {
                    proving_args.extend(vec![
                        String::from("--pinata-api-url"),
                        pinata_api_url.to_string(),
                    ]);
                }
                if let Some(ipfs_gateway_url) = &storage_provider_config.ipfs_gateway_url {
                    proving_args.extend(vec![
                        String::from("--ipfs-gateway-url"),
                        ipfs_gateway_url.to_string(),
                    ]);
                }
            }
            StorageProviderType::File => {
                proving_args.extend(vec![
                    String::from("--storage-provider"),
                    String::from("file"),
                ]);
                if let Some(file_path) = &storage_provider_config.file_path {
                    proving_args.extend(vec![
                        String::from("--file-path"),
                        file_path.to_str().unwrap().to_string(),
                    ]);
                }
            }
            _ => unimplemented!("Unknown storage provider."),
        }
        proving_args
    }
}

pub async fn run_boundless_client(
    args: MarketProviderConfig,
    storage: StorageProviderConfig,
    journal: ProofJournal,
    witness_frames: Vec<Vec<u8>>,
    stitched_proofs: Vec<Receipt>,
    proving_args: &ProvingArgs,
) -> Result<Receipt, ProvingError> {
    info!("Running boundless client.");
    let proof_journal = Journal::new(journal.encode_packed());

    // Override deployment configuration if set
    let market_deployment = args
        .boundless_chain_id
        .and_then(Deployment::from_chain_id)
        .or_else(|| {
            let mut builder = Deployment::builder();
            if let Some(boundless_market_address) = args.boundless_market_address {
                builder.boundless_market_address(boundless_market_address);
            };
            if let Some(boundless_verifier_router_address) =
                args.boundless_verifier_router_address
            {
                builder.verifier_router_address(boundless_verifier_router_address);
            };
            if let Some(boundless_set_verifier_address) = args.boundless_set_verifier_address {
                builder.set_verifier_address(boundless_set_verifier_address);
            };
            if let Some(boundless_stake_token_address) = args.boundless_stake_token_address {
                builder.stake_token_address(boundless_stake_token_address);
            };
            if let Some(boundless_order_stream_url) = args.boundless_order_stream_url.clone() {
                builder.order_stream_url(boundless_order_stream_url);
            };
            builder.build().ok()
        });

    // Instantiate client
    let boundless_client = Client::builder()
        .with_private_key(args.boundless_wallet_key)
        .with_rpc_url(args.boundless_rpc_url)
        .with_deployment(market_deployment)
        .with_storage_provider_config(&storage)
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?
        .build()
        .await
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;

    // Set the proof request requirements
    let requirements = Requirements::new(
        KAILUA_FPVM_ID,
        Predicate::digest_match(proof_journal.digest()),
    )
        // manually choose latest Groth16 receipt selector
        .with_selector((Selector::groth16_latest() as u32).into());

    // Check if an unexpired request had already been made recently
    let boundless_wallet_address = boundless_client.signer.as_ref().unwrap().address();
    let boundless_wallet_nonce = boundless_client
        .provider()
        .get_transaction_count(boundless_wallet_address)
        .await
        .context("get_transaction_count boundless_wallet_address")
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))? as u32;

    // Look back at prior transactions to avoid repeated requests
    for i in 0..args.boundless_lookback {
        if i > boundless_wallet_nonce {
            break;
        }
        let nonce = boundless_wallet_nonce.saturating_sub(i + 1);

        let request_id = RequestId::u256(boundless_wallet_address, nonce);
        info!("Looking back at txn w/ nonce {nonce} | request: {request_id:x}");

        let Ok((request, _)) = boundless_client
            .boundless_market
            .get_submitted_request(request_id, None)
            .await
        else {
            // No request for that nonce
            continue;
        };

        let request_status = boundless_client
            .boundless_market
            .get_status(request_id, Some(request.expires_at()))
            .await
            .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;

        if matches!(request_status, RequestStatus::Expired) {
            // We found a duplicate but it was expired
            continue;
        }

        // Skip unrelated request
        if request.requirements != requirements {
            continue;
        }

        info!("Found matching request already submitted!");

        if proving_args.skip_await_proof {
            warn!("Skipping awaiting proof on Boundless and exiting process.");
            std::process::exit(0);
        }

        return retrieve_proof(
            &boundless_client,
            request_id,
            args.boundless_order_check_interval,
            request.expires_at(),
        )
            .await
            .map_err(|e| ProvingError::OtherError(anyhow!(e)));
    }

    // Preflight execution to get cycle count
    info!("Preflighting execution.");
    let preflight_witness_frames = witness_frames.clone();
    let preflight_stitched_proofs = stitched_proofs.clone();
    let segment_limit = proving_args.segment_limit;
    let session_info = tokio::task::spawn_blocking(move || {
        let mut builder = ExecutorEnv::builder();
        // Set segment po2
        builder.segment_limit_po2(segment_limit);
        // Pass in witness data
        for frame in &preflight_witness_frames {
            builder.write_frame(frame);
        }
        // Pass in proofs
        for proof in &preflight_stitched_proofs {
            builder.write(proof)?;
        }
        let env = builder.build()?;
        let session_info = default_executor().execute(env, KAILUA_FPVM_ELF)?;
        Ok::<_, anyhow::Error>(session_info)
    })
        .await
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?
        .map_err(|e| ProvingError::ExecutionError(anyhow!(e)))?;

    // todo: remember this storage location to avoid duplicate uploads
    // Upload the ELF to the storage provider so that it can be fetched by the market.
    if boundless_client.storage_provider.is_none() {
        return Err(ProvingError::OtherError(anyhow!(
            "A storage provider is required to host the FPVM program and input."
        )));
    }
    let image_url = boundless_client
        .upload_program(KAILUA_FPVM_ELF)
        .await
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;
    info!("Uploaded image to {}", image_url);
    // Upload input
    let mut builder = GuestEnvBuilder::new();
    for frame in &witness_frames {
        builder = builder.write_frame(frame);
    }
    // Pass in proofs
    for proof in &stitched_proofs {
        builder = builder
            .write(proof)
            .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;
    }
    // Build final input
    let input = builder
        .build_vec()
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;
    let input_url = boundless_client
        .upload_input(&input)
        .await
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;
    info!("Uploaded input to {input_url}");

    let boundless_rpc_time = boundless_client
        .provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?
        .ok_or_else(|| ProvingError::OtherError(anyhow!("Failed to fetch latest block from Boundless RPC")))?
        .header
        .timestamp;

    let cycle_count = session_info
        .segments
        .iter()
        .map(|segment| 1 << segment.po2)
        .sum::<u64>();
    let segment_count = cycle_count.div_ceil(1_000_000) as f64;
    let cycles = U256::from(cycle_count);
    let mcycles = cycles.div_ceil(U256::from(1_000_000));
    let min_price = args.boundless_cycle_min_wei * cycles;
    let max_price = args.boundless_cycle_max_wei * cycles;
    let bid_delay_time = (args.boundless_order_bid_delay_factor * segment_count) as u64;
    let corrected_lock_timeout_factor =
        args.boundless_order_ramp_up_factor + args.boundless_order_lock_timeout_factor;
    let corrected_expiry_factor =
        corrected_lock_timeout_factor + args.boundless_order_expiry_factor;
    let request = boundless_client
        .new_request()
        .with_journal(proof_journal)
        .with_cycles(cycle_count)
        .with_program_url(image_url)
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?
        .with_input_url(input_url)
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?
        .with_requirements(requirements)
        .with_offer(
            OfferParams::builder()
                .min_price(min_price)
                .max_price(max_price)
                .bidding_start(boundless_rpc_time + bid_delay_time)
                .lock_stake(args.boundless_mega_mcycle_stake * mcycles)
                .ramp_up_period((args.boundless_order_ramp_up_factor * segment_count) as u32)
                .lock_timeout((corrected_lock_timeout_factor * segment_count) as u32)
                .timeout((corrected_expiry_factor * segment_count) as u32)
                .build()
                .map_err(|e| ProvingError::OtherError(anyhow!(e)))?,
        )
        .with_request_id(RequestId::new(boundless_wallet_address, boundless_wallet_nonce));

    // Send the request and wait for it to be completed.
    let (request_id, expires_at) = if args.boundless_order_stream_url.is_some() {
        info!("Submitting offchain request.");
        boundless_client
            .submit_offchain(request.clone())
            .await
            .context("Client::submit_offchain()")
            .map_err(|e| ProvingError::OtherError(anyhow!(e)))?
    } else {
        info!("Submitting onchain request.");
        boundless_client
            .submit_onchain(request.clone())
            .await
            .context("Client::submit_onchain()")
            .map_err(|e| ProvingError::OtherError(anyhow!(e)))?
    };

    if proving_args.skip_await_proof {
        warn!("Skipping awaiting proof on Boundless and exiting process.");
        std::process::exit(0);
    }

    retrieve_proof(
        &boundless_client,
        request_id,
        args.boundless_order_check_interval,
        expires_at,
    )
        .await
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))
}

pub async fn retrieve_proof(
    boundless_client: &Client,
    request_id: U256,
    interval: u64,
    expires_at: u64,
) -> anyhow::Result<Receipt> {
    // Wait for the request to be fulfilled by the market, returning the journal and seal.
    info!("Waiting for 0x{request_id:x} to be fulfilled");
    let (journal, seal) = boundless_client
        .wait_for_request_fulfillment(request_id, Duration::from_secs(interval), expires_at)
        .await?;

    let risc0_ethereum_contracts::receipt::Receipt::Base(receipt) =
        risc0_ethereum_contracts::receipt::decode_seal(seal, KAILUA_FPVM_ID, journal)?
    else {
        bail!("Did not receive an unaggregated receipt.");
    };

    Ok(*receipt)
}