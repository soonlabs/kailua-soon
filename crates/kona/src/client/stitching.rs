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

use crate::boot::StitchedBootInfo;
use crate::client::log;
use crate::driver::CachedDriver;
use crate::executor::Execution;
use crate::journal::ProofJournal;
use crate::kona::OracleL1ChainProvider;
use crate::precondition::Precondition;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, B256};
use anyhow::Context;
use kona_executor::L2BlockBuilder;
use kona_preimage::CommsClient;
use kona_proof::l2::OracleL2ChainProvider;
use kona_proof::{BootInfo, FlushableCache};
use risc0_zkvm::sha::Digestible;
use soon_derive::traits::{BlobProvider, ChainProvider};
use std::fmt::Debug;
use std::iter::zip;
use std::sync::Arc;
#[cfg(target_os = "zkvm")]
use {
    alloy_primitives::map::HashSet,
    risc0_zkvm::{serde::Deserializer, sha::Digest, Receipt},
    serde::Deserialize,
};

pub trait StitchingClient<
    E,
    O: CommsClient + FlushableCache + Send + Sync + Debug,
    B: BlobProvider + Send + Sync + Debug + Clone,
>
{
    /// Runs the Kailua client to transition the rollup state and combines the result with
    /// other proven contiguous state transitions to yield a single overarching
    /// `ProofJournal` and `Precondition`.
    ///
    /// The returned `BootInfo` instance is what was loaded by the Kona client.
    ///
    /// # Arguments
    ///
    /// * `proposal_data_hash` - The hash of the proposal blob precondition data.
    /// * `oracle` - The client for preloaded communication with the host environment.
    /// * `stream` - The client for streamed communication with the host.
    /// * `beacon` - The blob provider.
    /// * `fpvm_image_id` - A `B256` identifier for the FPVM image to associate with the operations performed.
    /// * `payout_recipient_address` - The Ethereum address (`Address`) where payout rewards are allocated.
    /// * `stitched_executions` - A nested vector of `Execution` objects containing precomputed execution
    ///   proofs to be stitched.
    /// * `derivation_cache`: An initial snapshot to load for the derivation pipeline.
    /// * `derivation_trace`: Whether to capture the final snapshot of the derivation pipeline in the precondition.
    /// * `stitched_preconditions`: A vector of `Precondition` objects for the stitched proofs.
    /// * `stitched_boot_info` - A vector of `StitchedBootInfo` objects describing proofs
    ///   to be stitched together.
    #[allow(clippy::too_many_arguments)]
    fn run_stitching_client(
        self,
        proposal_data_hash: B256,
        oracle: Arc<O>,
        stream: Arc<O>,
        beacon: B,
        fpvm_image_id: B256,
        payout_recipient_address: Address,
        stitched_executions: Vec<Vec<Execution>>,
        derivation_cache: Option<CachedDriver>,
        derivation_trace: bool,
        stitched_preconditions: Vec<Precondition>,
        stitched_boot_info: Vec<StitchedBootInfo>,
    ) -> (BootInfo, ProofJournal, Precondition)
    where
        <B as BlobProvider>::Error: Debug,
        E: L2BlockBuilder<OracleL2ChainProvider<O>, OracleL2ChainProvider<O>> + Send + Sync + Debug;
}

#[derive(Clone, Debug)]
pub struct KonaStitchingClient;

impl<
        O: CommsClient + FlushableCache + Send + Sync + Debug,
        B: BlobProvider + Send + Sync + Debug + Clone,
        E: L2BlockBuilder<OracleL2ChainProvider<O>, OracleL2ChainProvider<O>> + Send + Sync + Debug,
    > StitchingClient<E, O, B> for KonaStitchingClient
{
    fn run_stitching_client(
        self,
        proposal_data_hash: B256,
        oracle: Arc<O>,
        stream: Arc<O>,
        beacon: B,
        fpvm_image_id: B256,
        payout_recipient_address: Address,
        stitched_executions: Vec<Vec<Execution>>,
        derivation_cache: Option<CachedDriver>,
        derivation_trace: bool,
        stitched_preconditions: Vec<Precondition>,
        stitched_boot_info: Vec<StitchedBootInfo>,
    ) -> (BootInfo, ProofJournal, Precondition)
    where
        <B as BlobProvider>::Error: Debug,
    {
        // Queue up precomputed executions
        let (stitched_executions, execution_cache) = split_executions(stitched_executions);

        // Attempt to recompute the output hash at the target block number using kona
        log("RUN");
        let (boot, precondition) = crate::client::core::run_core_client(
            proposal_data_hash,
            oracle,
            stream.clone(),
            beacon,
            execution_cache,
            None,
            derivation_cache,
            derivation_trace.then(Default::default),
        )
        .expect("Failed to compute output hash.");

        // Verify proofs recursively for boundless composition
        #[cfg(target_os = "zkvm")]
        let proven_fpvm_journals = load_stitching_journals(fpvm_image_id);

        // Stitch recursively composed execution-only proofs
        stitch_executions(
            &boot,
            fpvm_image_id,
            payout_recipient_address,
            &stitched_executions,
            #[cfg(target_os = "zkvm")]
            &proven_fpvm_journals,
        );

        // Stitch recursively composed proofs
        kona_proof::block_on(stitch_boot_info(
            Some(stream),
            boot,
            fpvm_image_id,
            payout_recipient_address,
            precondition,
            stitched_preconditions,
            stitched_boot_info,
            #[cfg(target_os = "zkvm")]
            &proven_fpvm_journals,
        ))
        .expect("Failed to stitch boot info.")
    }
}

/// Loads and verifies stitching journals for a given FPVM image.
///
/// This function continuously reads receipts representing the proofs of computations from the
/// standard input (stdin). Each receipt is validated against the provided `fpvm_image_id`,
/// representing the image digest of the FPVM. Validated receipts' journal digests are stored
/// in a `HashSet` ensuring uniqueness. If deserialization of the receipt fails, the function
/// terminates and returns the set of proven journal digests.
///
/// # Parameters
/// - `fpvm_image_id`: A `B256` type identifier representing the hashed image ID of the FPVM.
///
/// # Returns
/// - A `HashSet<Digest>` containing the unique journal digests of all verified receipts.
///
/// # Behavior
/// 1. Converts the `fpvm_image_id` into a `Digest` for verification purposes.
/// 2. Reads receipts in a loop from the standard input until an `Err` occurs during deserialization.
///    - While reading receipts:
///      - Logs the verification process.
///      - Deserializes and verifies receipts against the provided `fpvm_image_id`.
///      - Inserts successfully verified journal digests into the `HashSet`.
/// 3. Logs the total number of successfully verified journal digests and exits with the result.
///
/// # Panics
/// Panics if:
/// - Receipt verification fails, indicating an invalid or tampered proof. The panic message will
///   include which journal digest's verification failed.
///
/// # Logging
/// - Logs "VERIFY" at the start of the method.
/// - Logs "VERIFY {journal_digest}" after calculating journal digests.
/// - Logs "PROOFS {count}" denoting the number of proven journal digests before exiting.
///
/// # Notes
/// - The `Receipt::deserialize` and `risc0_zkvm::guest::env::stdin` are used to process input
///   receipts.
/// - This function is designed for environments where proofs generated externally are verified
///   within the FPVM.
#[cfg(target_os = "zkvm")]
pub fn load_stitching_journals(fpvm_image_id: B256) -> HashSet<Digest> {
    log("VERIFY");

    let fpvm_image_id = Digest::from(fpvm_image_id.0);
    let mut proven_fpvm_journals = HashSet::with_hasher(Default::default());

    loop {
        let Ok(receipt) =
            Receipt::deserialize(&mut Deserializer::new(risc0_zkvm::guest::env::stdin()))
        else {
            log(&format!("PROOFS {}", proven_fpvm_journals.len()));
            break proven_fpvm_journals;
        };

        let journal_digest = receipt.journal.digest();
        log(&format!("VERIFY {journal_digest}"));

        // Validate RISC Zero receipts natively
        receipt
            .verify(fpvm_image_id)
            .expect("Failed to verify receipt for {journal_digest}.");

        proven_fpvm_journals.insert(journal_digest);
    }
}

/// Verifies the stitching journal of an FPVM image.
///
/// This function checks the validity of a journal based on its digest and the existing
/// set of proven FPVM journal digests. The behavior of this function depends on the
/// target OS being `zkvm`. If the journal's digest exists in the set of verified digests,
/// it logs that the digest was found. Otherwise, it assumes the journal and attempts to
/// verify it using the RISC Zero ZKVM environment.
///
/// # Parameters
/// - `_fpvm_image_id`: The ID of the FPVM image represented as a `B256` hash. This
///   ID is used during the journal verification process.
/// - `_proof_journal`: The serialized proof journal as a `Vec<u8>`. It serves as
///   the data to be verified.
/// - `proven_fpvm_journals`: A reference to a `HashSet` of digests (of type `Digest`)
///   containing the previously verified journals. This parameter is only used when
///   the target OS is `zkvm`.
///
/// # Logs
/// - Logs a message indicating whether the given journal digest was "FOUND" in the proven
///   set or "ASSUME" if it is not present.
///
/// # Panics
/// - If the verification process fails (i.e., the journal does not match the
///   expected criteria for verification), the function will panic with the message:
///   `"Failed to verify stitched journal assumption"`.
pub fn verify_stitching_journal(
    _fpvm_image_id: B256,
    _proof_journal: Vec<u8>,
    #[cfg(target_os = "zkvm")] proven_fpvm_journals: &HashSet<Digest>,
) {
    #[cfg(target_os = "zkvm")]
    {
        let journal_digest = _proof_journal.digest();
        if proven_fpvm_journals.contains(&journal_digest) {
            crate::client::log(&format!("FOUND {journal_digest}"));
        } else {
            crate::client::log(&format!("ASSUME {journal_digest}"));
            risc0_zkvm::guest::env::verify(_fpvm_image_id.0, &_proof_journal)
                .expect("Failed to verify stitched journal assumption");
        }
    }
}

/// Splits a provided two-dimensional vector of `Execution` objects into two separate structures:
/// - A nested two-dimensional vector where each inner `Execution` is wrapped in an `Arc`.
/// - A flattened vector containing all the `Execution` objects, each wrapped in an `Arc`.
///
/// This function is useful for scenarios where you want to maintain the original structure
/// but also need a separate flattened cache to quickly access all `Execution` objects.
///
/// # Arguments
///
/// * `stitched_executions` - A two-dimensional vector of `Execution` objects (`Vec<Vec<Execution>>`)
///   representing grouped and stitched executions.
///
/// # Returns
///
/// A tuple containing:
/// 1. A two-dimensional vector (`Vec<Vec<Arc<Execution>>>`) where each `Execution` is wrapped in an `Arc`.
/// 2. A flattened vector (`Vec<Arc<Execution>>`) representing a cache of all `Execution` objects.
pub fn split_executions(
    stitched_executions: Vec<Vec<Execution>>,
) -> (Vec<Vec<Arc<Execution>>>, Vec<Arc<Execution>>) {
    let stitched_executions = stitched_executions
        .into_iter()
        .map(|trace| trace.into_iter().map(Arc::new).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let execution_cache = stitched_executions
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    (stitched_executions, execution_cache)
}

/// Stitches a collection of execution traces into a cohesive proof journal and validates the results.
/// This function ensures the integrity of execution traces and their compliance with the rollup configuration.
///
/// # Parameters
/// - `boot`: A reference to the `BootInfo` structure containing the rollup's configuration and state information.
/// - `fpvm_image_id`: The unique identifier of the FPVM (Fault-Proof Virtual Machine) image being used for proofs.
/// - `payout_recipient_address`: The address to receive the payout as a result of the execution.
/// - `stitched_executions`: A reference to a vector of vectors containing execution traces. Each inner vector represents
///   a sequence of linked execution steps (`Execution` objects).
/// - `proven_fpvm_journals` (*conditional*): A reference to a set of `Digest` values representing proven
///   journals from the FPVM. Only available when compiled for `zkvm` target (`#[cfg(target_os = "zkvm")]`).
///
/// # Behavior
/// - When the `boot.l1_head` is zero, it represents a special case where only one batch of execution is validated
///   by the Kailua client. If more than one batch is found, the function panics.
/// - Validates the `receipts_root` of each execution in all traces by comparing it with the computed root value
///   based on the execution result, rollup configuration, and payload attributes' timestamp.
/// - Constructs an expected proof journal for each execution trace, which includes precondition and configuration
///   hashes, and other state values derived from the execution trace (e.g., output roots and block numbers).
/// - When the system is targeting `zkvm`, the proof journal is verified using the `proven_fpvm_journals`.
///
/// # Panics
/// - When `boot.l1_head` is zero but the number of `stitched_executions` exceeds 1.
/// - When an execution trace is empty (used in `.first()` or `.last()` calls without valid elements).
pub fn stitch_executions(
    boot: &BootInfo,
    fpvm_image_id: B256,
    payout_recipient_address: Address,
    stitched_executions: &Vec<Vec<Arc<Execution>>>,
    #[cfg(target_os = "zkvm")] proven_fpvm_journals: &HashSet<Digest>,
) {
    let config_hash = crate::config::config_hash(&boot.rollup_config);
    // When running an execution-only proof, we may only have one batch validated by the kailua client
    if boot.l1_head.is_zero() {
        assert_eq!(1, stitched_executions.len());
        return;
    };
    // Otherwise, we validate that all cached executions have corresponding exec-only proofs
    for execution_trace in stitched_executions {
        let precondition_hash =
            crate::precondition::execution::exec_precondition_hash(execution_trace.as_slice());
        // Construct expected proof journal
        let encoded_journal = ProofJournal::new_stitched(
            fpvm_image_id,
            payout_recipient_address,
            precondition_hash,
            B256::from(config_hash),
            &StitchedBootInfo {
                l1_head: B256::ZERO,
                agreed_l2_output_root: execution_trace
                    .first()
                    .expect("Empty execution trace")
                    .agreed_output,
                claimed_l2_output_root: execution_trace
                    .last()
                    .expect("Empty execution trace")
                    .claimed_output,
                claimed_l2_block_number: execution_trace
                    .last()
                    .expect("Empty execution trace")
                    .artifacts
                    .block_info
                    .block_info
                    .number,
            },
        )
        .encode_packed();
        // Require an execution-only proof for the entire batch
        verify_stitching_journal(
            fpvm_image_id,
            encoded_journal,
            #[cfg(target_os = "zkvm")]
            proven_fpvm_journals,
        )
    }
}

/// Stitches multiple boot information records into a unified `ProofJournal`.
///
/// This function consolidates and verifies multiple bootstrapping records, validating their
/// integrity and creating a coherent journal that reflects the intermediate states and outputs
/// of the bootstrapping process.
///
/// NOTE: This method does not support combining execution-only proofs.
///
/// # Arguments
///
/// * `boot` - A reference to the base `BootInfo` structure used as the initial data point.
/// * `fpvm_image_id` - A 256-bit identifier representing the FPVM image being used.
/// * `payout_recipient_address` - The Ethereum address to which payouts should be sent.
/// * `precondition_hash` - A 256-bit hash representing the preconditions required for stitching.
/// * `stitched_boot_info` - A vector of `StitchedBootInfo` objects that are incrementally stitched
///   into the `ProofJournal`.
/// * `proven_fpvm_journals` - (Optional, only on `zkvm` platforms) A reference to a set of
///   precomputed and verified FPVM journal digests used for proof verification.
///
/// # Returns
///
/// A `ProofJournal` object that reflects the final stitched state after processing
/// all input records.
///
/// # Panics
///
/// This function will panic in the following scenarios:
///
/// 1. **Equivalence Check Failure**: If the `l1_head` values in the current and stitched boots
///    are inconsistent.
/// 2. **Progress Check Failure**: If there is no progress between the `agreed_l2_output_root` and
///    `claimed_l2_output_root` of a `stitched_boot` object.
/// 3. **Proof Assumption Failure**: If the stitching proof journal fails the `verify_stitching_journal`
///    check.
/// 4. **Non-contiguous Stitching**: If the claimed and agreed L2 output roots cannot be matched
///    in a forward or backward stitching configuration.
/// 5. **Execution-only Records**: If the combination of execution-only boot infos is attempted.
///
/// # Stitching Logic
///
/// 1. The function initializes a `ProofJournal` object using the base `BootInfo` structure and
///    additional parameters.
/// 2. For each `StitchedBootInfo` object in `stitched_boot_info`:
///     - Verify the equivalence of `l1_head`.
///     - Ensure progress is made between `agreed_l2_output_root` and `claimed_l2_output_root`.
///     - Validate the proof associated with the stitching via the `verify_stitching_journal` function.
///     - Perform continuity checks and update the journal in a forward or backward stitching
///       configuration. If stitching is non-contiguous, the function will panic.
///
/// # Platform-specific Behavior
///
/// * On `zkvm` platforms, the function requires access to `proven_fpvm_journals` to verify stitching
///   proofs. On other platforms, the verification step is omitted.
pub async fn stitch_boot_info<O: CommsClient + FlushableCache + Send + Sync + Debug>(
    stream: Option<Arc<O>>,
    boot: BootInfo,
    fpvm_image_id: B256,
    payout_recipient_address: Address,
    mut precondition: Precondition,
    stitched_preconditions: Vec<Precondition>,
    stitched_boot_infos: Vec<StitchedBootInfo>,
    #[cfg(target_os = "zkvm")] proven_fpvm_journals: &HashSet<Digest>,
) -> anyhow::Result<(BootInfo, ProofJournal, Precondition)> {
    // Equal inputs
    assert_eq!(stitched_preconditions.len(), stitched_boot_infos.len());

    // Instantiate oracle-backed providers
    let mut l1_provider = match stream {
        Some(stream) => Some(OracleL1ChainProvider::new(boot.l1_head, stream).await?),
        None => None,
    };

    // Instantiate base proof journal for validating stitched proofs
    let mut journal = ProofJournal::new(
        fpvm_image_id,
        payout_recipient_address,
        B256::ZERO, // Precondition digest will be finalized below
        &boot,
    );

    // Stitch boot info instances
    let mut l1_head_number = match l1_provider.as_mut() {
        Some(provider) if !boot.l1_head.is_zero() => Some(
            provider
                .header_by_hash(boot.l1_head)
                .await
                .context("boot header_by_hash")?
                .number,
        ),
        _ => None,
    };
    for (stitched_boot, stitched_precondition) in zip(stitched_boot_infos, stitched_preconditions) {
        // Check if stitched l1 head is in the same chain
        if boot.l1_head.is_zero() || stitched_boot.l1_head.is_zero() {
            unimplemented!("Stitching boot infos of execution-only proofs is not supported.");
        } else if let Some(l1_provider) = l1_provider.as_mut() {
            // Retrieve the full header, which must then be verified to be from the same chain
            let stitched_l1_header = l1_provider
                .header_by_hash(stitched_boot.l1_head)
                .await
                .context("header_by_hash")?;
            // Ensure non-increasing derivation heads
            let l1_head_number = l1_head_number.as_mut().unwrap();
            assert!(stitched_l1_header.number <= *l1_head_number);
            *l1_head_number = stitched_l1_header.number;
            // Ensure that querying the oracle by the header number yields the same header hash
            assert_eq!(
                l1_provider
                    .block_info_by_number(BlockNumberOrTag::Number(stitched_l1_header.number))
                    .await
                    .context("block_info_by_number")?
                    .hash,
                stitched_boot.l1_head
            );
        }
        // Require equivalence in proposal precondition
        assert_eq!(
            precondition.proposal_blobs,
            stitched_precondition.proposal_blobs
        );
        // Require backward stitching (stitched proof leads to current journal state)
        assert_eq!(
            stitched_boot.claimed_l2_output_root,
            journal.agreed_l2_output_root
        );
        // Stitched boot's trace must be our cache
        assert_eq!(
            precondition.derivation_cache,
            stitched_precondition.derivation_trace
        );
        // Update our initial l2 output root to that of the stitched boot
        journal.agreed_l2_output_root = stitched_boot.agreed_l2_output_root;
        // Update our cache to be that of the backwards stitched boot
        precondition.derivation_cache = stitched_precondition.derivation_cache;
        // Require derivation proof for stitched boot
        verify_stitching_journal(
            fpvm_image_id,
            ProofJournal::new_stitched(
                fpvm_image_id,
                payout_recipient_address,
                B256::new(stitched_precondition.digest().into()),
                journal.config_hash,
                &stitched_boot,
            )
            .encode_packed(),
            #[cfg(target_os = "zkvm")]
            proven_fpvm_journals,
        );
    }

    // Update the final precondition hash
    journal.precondition_hash = B256::new(precondition.digest().into());

    // Report final precondition
    log("STITCHED");
    log(&format!("{journal:?}"));
    log(&format!("{precondition:?}"));

    Ok((boot, journal, precondition))
}
