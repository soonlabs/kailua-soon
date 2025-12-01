// Copyright 2024, 2025 RISC Zero, Inc.
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

use crate::client;
use crate::client::log;
use crate::config::config_hash;
use crate::driver::CachedDriver;
use crate::executor::{new_execution_cursor, CachedExecutor, Execution};
use crate::kona::OracleL1ChainProvider;
use crate::oracle::local::LocalOnceOracle;
use crate::precondition::execution::exec_precondition_hash;
use crate::precondition::{proposal, Precondition};
use alloy_primitives::B256;
use anyhow::{bail, Context};
use kona_driver::{Driver, Executor};
use kona_executor::{L2BlockBuilder, StatelessL2Builder, TrieDBProvider};
use kona_mpt::TrieHinter;
use kona_preimage::{CommsClient, PreimageKey};
use kona_proof::errors::OracleProviderError;
use kona_proof::executor::KonaExecutor;
use kona_proof::l1::OracleDaProvider;
use kona_proof::l1::OraclePipeline;
use kona_proof::l2::{CursorSetter, OracleL2ChainProvider};
use kona_proof::sync::new_oracle_pipeline_cursor;
use kona_proof::{BootInfo, FlushableCache, HintType};
use risc0_zkvm::sha::Digestible;
use soon_derive::prelude::{ChainProvider, DAProvider};
use soon_derive::sources::DAServerSource;
use soon_derive::traits::{BlobProvider, L2ChainProvider};
use soon_primitives::blocks::L2BlockHeader;
use soon_primitives::output_root::OutputRoot;
use std::fmt::Debug;
use std::mem::take;
use std::sync::{Arc, Mutex};
use tracing::info;

/// Initializes the L1, L2, and DA providers for the core client.
///
/// This function extracts the common provider initialization logic that was duplicated
/// across multiple functions in this module.
///
/// # Arguments
/// * `oracle` - The oracle client for communicating with the host environment
/// * `stream` - The stream client for streamed communication with the host
///
/// # Returns
/// A tuple containing the initialized L1 provider, L2 provider, and DA provider
async fn initialize_providers<O>(
    oracle: Arc<O>,
    stream: Arc<O>,
) -> anyhow::Result<(
    OracleL1ChainProvider<O>,
    OracleL2ChainProvider<O>,
    OracleDaProvider<O>,
)>
where
    O: CommsClient + FlushableCache + Send + Sync + Debug,
{
    let boot = BootInfo::load(oracle.as_ref())
        .await
        .context("BootInfo::load")?;
    let rollup_config = Arc::new(boot.rollup_config.clone());

    log("SAFE HEAD HASH");
    let safe_head_hash = fetch_safe_head_hash(oracle.as_ref(), boot.agreed_l2_output_root).await?;

    let l1_provider = OracleL1ChainProvider::new(boot.l1_head, stream)
        .await
        .context("new oracle l1 chain provider failed")?;
    let l2_provider =
        OracleL2ChainProvider::new(safe_head_hash, rollup_config.clone(), oracle.clone());
    let da_provider = OracleDaProvider::new(oracle);

    Ok((l1_provider, l2_provider, da_provider))
}

/// Runs the Kailua client to drive rollup state transition derivation using Kona.
///
/// # Arguments
/// * `precondition_validation_data_hash` - A 256-bit hash used for fetching precondition data.
/// * `oracle` - The client for communicating with the host environment.
/// * `stream` - The client for streamed communication with the host.
/// * `beacon` - An instance of the blob provider.
/// * `execution_cache` - A vector of cached executions to reuse.
/// * `collection_target` - An optional target to dump uncached executions.
///
/// # Returns
/// A result containing a tuple (`BootInfo`, `B256`) upon success, or an error of type `anyhow::Error`.
/// - `BootInfo` contains essential configuration information for bootstrapping the rollup client.
/// - `B256` represents a 256-bit hash of the computed output state.
///
/// # Errors
/// This function can return an error in any of the following cases:
/// * Failure to load `BootInfo`.
/// * Invalid `claimed_l2_block_number` value compared to the safe L2 head number.
/// * Assertion failures during execution trace validation, block derivations, and outputs validation.
/// * Insufficient L1 data to derive L2 output roots for the claimed block height.
///
/// # Workflow
///
/// ## 1. Bootstrapping & Safe Head Validation
/// - Loads `BootInfo` from the oracle.
/// - Fetches the safe head hash and constructs chain providers for both L1 and L2.
/// - Validates that the claimed L2 block number is greater than or equal to the L2 safe head.
///
/// ## 2. Execution Caching
/// - If the L1 head is a zero hash, the function operates in "execution only" mode:
///     - Initializes the execution cursor and uses a `KonaExecutor` for execution validation.
///     - Validates the consistency of execution traces against the expected results derived from `execution_cache`.
///
/// ## 3. Derivation and Execution
/// - Loads precondition data based on the provided hash, if any.
/// - Initializes the pipeline cursor and an `OraclePipeline`.
/// - Combines execution caching with pipeline-driven iteration to derive L2 outputs incrementally until the claimed L2 height:
///     - Validates outputs, ensuring sufficient L1 data exists for subsequent derivations.
///     - Adjusts the executor state for consecutive computation and output production.
///     - Logs the progress and appends derived output roots.
///
/// ## 4. Final Validation & Output
/// - Verifies the computed outputs:
///     - Ensures the final output hash matches the claimed L2 output root.
///     - Handles insufficient data to derive output roots by returning a matching zero hash.
pub fn run_core_client<
    O: CommsClient + FlushableCache + Send + Sync + Debug,
    B: BlobProvider + Send + Sync + Debug + Clone,
>(
    precondition_validation_data_hash: B256,
    oracle: Arc<O>,
    stream: Arc<O>,
    beacon: B,
    execution_cache: Vec<Arc<Execution>>,
    execution_trace: Option<Arc<Mutex<Vec<Execution>>>>,
    derivation_cache: Option<CachedDriver>,
    derivation_trace: Option<Arc<Mutex<Option<CachedDriver>>>>,
) -> anyhow::Result<(BootInfo, Precondition)>
where
    <B as BlobProvider>::Error: Debug,
{
    let clone_oracle = oracle.clone();
    let (l1_provider, l2_provider, da_provider) =
        kona_proof::block_on(async move { initialize_providers(clone_oracle, stream).await })?;

    run_core_client_ex::<
        StatelessL2Builder<OracleL2ChainProvider<O>, OracleL2ChainProvider<O>>,
        O,
        B,
        OracleL1ChainProvider<O>,
        OracleL2ChainProvider<O>,
        OracleDaProvider<O>,
    >(
        precondition_validation_data_hash,
        oracle,
        beacon,
        l1_provider,
        l2_provider,
        da_provider,
        execution_cache,
        execution_trace,
        derivation_cache,
        derivation_trace,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_core_client_ex<
    E,
    O: CommsClient + FlushableCache + Send + Sync + Debug,
    B: BlobProvider + Send + Sync + Debug + Clone,
    L1: ChainProvider + Send + Sync + Debug + Clone,
    L2: TrieDBProvider + TrieHinter + L2ChainProvider + CursorSetter + Send + Sync + Debug + Clone,
    DA: DAProvider + Send + Sync + Debug + Clone,
>(
    proposal_data_hash: B256,
    oracle: Arc<O>,
    mut beacon: B,
    mut l1_provider: L1,
    mut l2_provider: L2,
    da_provider: DA,
    execution_cache: Vec<Arc<Execution>>,
    execution_trace: Option<Arc<Mutex<Vec<Execution>>>>,
    derivation_cache: Option<CachedDriver>,
    derivation_trace: Option<Arc<Mutex<Option<CachedDriver>>>>,
) -> anyhow::Result<(BootInfo, Precondition)>
where
    <B as BlobProvider>::Error: Debug,
    E: L2BlockBuilder<L2, L2> + Send + Sync + Debug,
    L2: L2ChainProvider<Error = OracleProviderError>,
    L1: ChainProvider<Error = OracleProviderError>,
{
    let oracle = Arc::new(LocalOnceOracle::new(oracle));
    kona_proof::block_on(async move {
        ////////////////////////////////////////////////////////////////
        //                          PROLOGUE                          //
        ////////////////////////////////////////////////////////////////
        log("BOOT");
        let boot = BootInfo::load(oracle.as_ref())
            .await
            .context("BootInfo::load")?;
        log(&format!("{:?} L1_HEAD", boot.l1_head));
        log(&format!("{:?} L2_AGREED", boot.agreed_l2_output_root));
        log(&format!(
            "{:?} L2_CLAIMED (#{})",
            boot.claimed_l2_output_root, boot.claimed_l2_block_number
        ));
        let rollup_config = Arc::new(boot.rollup_config.clone());
        log(&format!("rollup_config: {:?}", rollup_config));
        log(&format!(
            "rollup_config_hash: {:?}",
            config_hash(&boot.rollup_config)
        ));

        // The claimed L2 block number must be greater than or equal to the L2 safe head.
        // Fetch the safe head's block header.
        log("SAFE HEAD");
        let safe_head = l2_provider
            .l2_block_info_by_number(boot.agreed_l2_block_number)
            .await?;
        let safe_head_output =
            fetch_safe_l2_output(oracle.as_ref(), boot.agreed_l2_output_root).await?;
        let safe_head_header = L2BlockHeader {
            block_info: safe_head.block_info,
            account_root: safe_head_output.state_root,
            widthdraw_root: safe_head_output.bridge_storage_root,
        };
        log("SAFE HEAD done");

        if boot.claimed_l2_block_number < safe_head_header.block_info.number {
            bail!("Invalid claim: Safe l2 head block number below claimed l2 block number.");
        }
        let safe_head_number = safe_head_header.block_info.number;
        info!(
            "SAFE HEAD number: {}, claimed_l2_block_number: {}",
            safe_head_number, boot.claimed_l2_block_number
        );
        let expected_output_count = (boot.claimed_l2_block_number - safe_head_number) as usize;

        ////////////////////////////////////////////////////////////////
        //                     EXECUTION CACHING                      //
        ////////////////////////////////////////////////////////////////
        if boot.l1_head.is_zero() {
            log("EXECUTION ONLY");
            let cursor =
                new_execution_cursor(rollup_config.as_ref(), safe_head_header, &mut l2_provider)
                    .await
                    .context("new_execution_cursor")?;
            l2_provider.set_cursor(cursor.clone());

            let mut kona_executor = KonaExecutor::<_, _, E>::new(
                rollup_config.clone(),
                l2_provider.clone(),
                l2_provider.clone(),
                None,
            );
            kona_executor.update_safe_head(safe_head_header)?;

            // Validate expected block count
            assert_eq!(expected_output_count, execution_cache.len());

            // Validate non-empty execution trace
            assert!(!execution_cache.is_empty());

            // Calculate precondition hash
            let execution_trace_hash = exec_precondition_hash(execution_cache.as_slice());

            // Validate terminating block number
            assert_eq!(
                execution_cache
                    .last()
                    .unwrap()
                    .artifacts
                    .block_info
                    .block_info
                    .number,
                boot.claimed_l2_block_number
            );

            let mut latest_output_root = boot.agreed_l2_output_root;
            // Validate executed chain
            for execution in execution_cache {
                info!(
                    "enter execution {}/{}",
                    execution.artifacts.block_info.block_info.number, boot.claimed_l2_block_number
                );
                // Unpack [Execution]
                let Execution {
                    agreed_output,
                    attributes,
                    artifacts,
                    claimed_output,
                } = execution.as_ref();
                // Verify initial state
                assert_eq!(agreed_output, &latest_output_root);
                // Verify transition
                let executor_result = kona_executor.execute_payload(attributes.clone()).await?;
                latest_output_root = kona_executor
                    .compute_output_root()
                    .context("compute_output_root: Verify post state")?;

                // check l2 header
                assert_eq!(artifacts.block_info, executor_result.block_info);
                assert_eq!(artifacts.state_root, executor_result.state_root);
                //TODO check result
                // assert_eq!(
                //     execution.artifacts.execution_result,
                //     executor_result.execution_result
                // );

                // Update state
                kona_executor.update_safe_head(L2BlockHeader {
                    block_info: artifacts.block_info.block_info,
                    account_root: executor_result.state_root,
                    widthdraw_root: executor_result.withdraw_root,
                })?;
                // Verify post state
                assert_eq!(claimed_output, &latest_output_root);
                log(&format!(
                    "OUTPUT: {}/{}",
                    artifacts.block_info.block_info.number, boot.claimed_l2_block_number
                ));
            }

            // Validate claimed_l2_output_root against latest_output_root
            assert_eq!(boot.claimed_l2_output_root, latest_output_root);
            // Return result
            return Ok((
                boot,
                Precondition::default().execution(execution_trace_hash),
            ));
        }

        ////////////////////////////////////////////////////////////////
        //                   DERIVATION & EXECUTION                   //
        ////////////////////////////////////////////////////////////////
        log("PRECONDITION");
        let proposal_precondition_data =
            proposal::load_proposal_data(proposal_data_hash, oracle.clone(), &mut beacon)
                .await
                .context("load_precondition_data")?;

        log("DERIVATION & EXECUTION");
        // Create a new derivation driver with the given boot information and oracle.
        let cursor = new_oracle_pipeline_cursor(
            rollup_config.as_ref(),
            safe_head_header,
            &mut l1_provider,
            &mut l2_provider,
        )
        .await
        .context("new_oracle_pipeline_cursor")?;
        l2_provider.set_cursor(cursor.clone());

        let da_source = DAServerSource::new(
            l1_provider.clone(),
            da_provider,
            rollup_config.batch_inbox_address,
        );
        // Load the Kailua executor with caching support
        let cached_executor: CachedExecutor<KonaExecutor<_, _, E>> = CachedExecutor::new(
            execution_cache,
            rollup_config.clone(),
            l2_provider.clone(),
            l2_provider.clone(),
            execution_trace,
        );
        // Resume from cached derivation pipeline or start a new one
        let (derivation_cache_hash, mut driver) = match derivation_cache {
            None => (
                B256::ZERO,
                Driver::new(
                    cursor.clone(),
                    cached_executor,
                    OraclePipeline::new(
                        rollup_config.clone(),
                        cursor,
                        oracle.clone(),
                        da_source,
                        l1_provider.clone(),
                        l2_provider.clone(),
                    )
                    .await
                    .context("OraclePipeline::new")?,
                ),
            ),
            Some(cached_driver) => (
                B256::new(cached_driver.digest().into()),
                cached_driver.uncache(
                    cached_executor,
                    rollup_config.clone(),
                    cursor,
                    oracle.clone(),
                    da_source,
                    l1_provider.clone(),
                    l2_provider.clone(),
                ),
            ),
        };

        // Run the derivation pipeline until we are able to produce the output root of the claimed
        // L2 block.
        let mut derived_output_roots = Vec::with_capacity(expected_output_count);
        for starting_block in safe_head_number..boot.claimed_l2_block_number {
            // Advance to the next target
            let (output_block, output_root) = driver
                .advance_to_target(&boot.rollup_config, Some(starting_block + 1))
                .await
                .context("advance_to_target")?;
            // Stop if nothing new was derived
            if output_block.block_info.number == starting_block {
                // No progress implies that there is insufficient L1 data available to produce
                // an L2 output root at this L2 height
                log("HALT");
                break;
            } else {
                log(&format!(
                    "OUTPUT: {}/{}",
                    output_block.block_info.number, boot.claimed_l2_block_number
                ));
            }
            // Append newly computed output root
            derived_output_roots.push(output_root);
        }

        ////////////////////////////////////////////////////////////////
        //                          EPILOGUE                          //
        ////////////////////////////////////////////////////////////////
        client::log("EPILOGUE");

        // Record derivation driver state
        let derivation_trace_hash = derivation_trace
            .map(|trace| {
                let derivation_trace = CachedDriver::from(driver);
                let trace_digest = B256::new(derivation_trace.digest().into());
                log(&format!("DERIVATION TRACE {trace_digest}"));
                let _ = trace.lock().unwrap().insert(derivation_trace);
                trace_digest
            })
            .unwrap_or_default();

        // Record intermediate output commitment precondition
        let proposal_precondition_hash = proposal_precondition_data
            .map(|(proposal_precondition, blobs)| {
                proposal::validate_proposal_precondition(
                    proposal_precondition,
                    blobs,
                    safe_head_number,
                    &derived_output_roots,
                )
            })
            .unwrap_or(Ok(B256::ZERO))
            .context("validate_precondition")?;

        // Compile final [Precondition]
        let precondition = Precondition::default()
            .proposal(proposal_precondition_hash)
            .derivation(derivation_cache_hash, derivation_trace_hash);

        // Compile the final [BootInfo]
        let claimed_l2_block_number = safe_head_number + derived_output_roots.len() as u64;
        let claimed_l2_output_root = derived_output_roots
            .pop()
            .unwrap_or(boot.agreed_l2_output_root);
        let boot = BootInfo {
            claimed_l2_output_root,
            claimed_l2_block_number,
            ..boot
        };

        // Return results
        Ok((boot, precondition))
    })
}

/// Fetches the safe head hash of the L2 chain based on the agreed upon L2 output root in the
/// [BootInfo].
pub async fn fetch_safe_head_hash<O>(
    caching_oracle: &O,
    agreed_l2_output_root: B256,
) -> Result<B256, OracleProviderError>
where
    O: CommsClient,
{
    let mut output_preimage = [0u8; 128];
    HintType::StartingL2Output
        .with_data(&[agreed_l2_output_root.as_ref()])
        .send(caching_oracle)
        .await?;
    caching_oracle
        .get_exact(
            PreimageKey::new_keccak256(*agreed_l2_output_root),
            output_preimage.as_mut(),
        )
        .await?;

    output_preimage[96..128]
        .try_into()
        .map_err(OracleProviderError::SliceConversion)
}

/// Fetches the safe header of the L2 chain based on the agreed upon L2 output root in the
/// [BootInfo].
pub async fn fetch_safe_l2_output<O>(
    caching_oracle: &O,
    agreed_l2_output_root: B256,
) -> Result<OutputRoot, OracleProviderError>
where
    O: CommsClient,
{
    let mut output_preimage = [0u8; 128];
    HintType::StartingL2Output
        .with_data(&[agreed_l2_output_root.as_ref()])
        .send(caching_oracle)
        .await?;
    caching_oracle
        .get_exact(
            PreimageKey::new_keccak256(*agreed_l2_output_root),
            output_preimage.as_mut(),
        )
        .await?;

    let state_root = output_preimage[32..64]
        .try_into()
        .map_err(OracleProviderError::SliceConversion)?;
    let bridge_storage_root = output_preimage[64..96]
        .try_into()
        .map_err(OracleProviderError::SliceConversion)?;
    let block_hash = output_preimage[96..128]
        .try_into()
        .map_err(OracleProviderError::SliceConversion)?;
    Ok(OutputRoot {
        state_root,
        bridge_storage_root,
        block_hash,
    })
}

/// Recovers a continuous execution trace from the collection target
pub fn recover_collected_executions(
    collection_target: Arc<Mutex<Vec<Execution>>>,
    claimed_l2_output_root: B256,
) -> Vec<Execution> {
    let mut executions = collection_target.lock().unwrap();
    for i in 1..executions.len() {
        executions[i - 1].claimed_output = executions[i].agreed_output;
    }
    if let Some(last_exec) = executions.last_mut() {
        last_exec.claimed_output = claimed_l2_output_root;
    }
    take::<Vec<Execution>>(executions.as_mut())
}
