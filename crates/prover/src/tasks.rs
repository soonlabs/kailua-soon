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

use crate::args::ProveArgs;
use crate::driver::{driver_file_name, signal_derivation_trace, try_read_driver};
use crate::kv::RWLKeyValueStore;
use crate::proof::{proof_file_name, read_bincoded_file};
use crate::ProvingError;
use alloy::providers::Provider;
use alloy_primitives::B256;
use anyhow::{anyhow, Context};
use async_channel::{Receiver, Sender};
use human_bytes::human_bytes;
use kailua_soon_kona::boot::StitchedBootInfo;
use kailua_soon_kona::client::stitching::{split_executions, stitch_boot_info};
use kailua_soon_kona::config::config_hash;
use kailua_soon_kona::driver::CachedDriver;
use kailua_soon_kona::executor::Execution;
use kailua_soon_kona::journal::ProofJournal;
use kailua_soon_kona::oracle::vec::VecOracle;
use kailua_soon_kona::precondition::execution::exec_precondition_hash;
use kailua_soon_kona::precondition::Precondition;
use kailua_sync::retry_res_ctx_timeout;
use kona_proof::BootInfo;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::trace::Tracer;
use risc0_zkvm::sha::Digestible;
use risc0_zkvm::Receipt;
use soon_derive::traits::L2ChainProvider;
use soon_primitives::rollup_config::SoonRollupConfig;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::convert::identity;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct CachedTask {
    pub args: ProveArgs,
    pub rollup_config: SoonRollupConfig,
    pub disk_kv_store: Option<RWLKeyValueStore>,
    pub precondition: Precondition,
    pub proposal_data_hash: B256,
    pub stitched_executions: Vec<Vec<Execution>>,
    pub derivation_cache: Option<CachedDriver>,
    pub derivation_trace: Option<Sender<CachedDriver>>,
    pub stitched_preconditions: Vec<Precondition>,
    pub stitched_boot_info: Vec<StitchedBootInfo>,
    pub stitched_proofs: Vec<Receipt>,
    pub prove_snark: bool,
    pub force_attempt: bool,
    pub seek_proof: bool,
}

impl PartialEq for CachedTask {
    fn eq(&self, other: &Self) -> bool {
        self.args.eq(&other.args)
    }
}

impl Eq for CachedTask {}

impl PartialOrd for CachedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CachedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        self.args.cmp(&other.args)
    }
}

pub type OneshotResultResponse = (Receipt, Precondition);

#[derive(Debug)]
pub struct OneshotResult {
    pub cached_task: CachedTask,
    pub result: Result<OneshotResultResponse, ProvingError>,
}

impl PartialEq for OneshotResult {
    fn eq(&self, other: &Self) -> bool {
        self.cached_task.eq(&other.cached_task)
    }
}

impl Eq for OneshotResult {}

impl PartialOrd for OneshotResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OneshotResult {
    fn cmp(&self, other: &Self) -> Ordering {
        self.cached_task.cmp(&other.cached_task)
    }
}

#[derive(Debug)]
pub struct Oneshot {
    pub cached_task: CachedTask,
    pub result_sender: Sender<OneshotResult>,
}

/// Indefinitely processes incoming [Oneshot] messages via the provided channel
pub async fn handle_oneshot_tasks(task_receiver: Receiver<Oneshot>) -> anyhow::Result<()> {
    loop {
        let Oneshot {
            cached_task,
            result_sender,
        } = task_receiver
            .recv()
            .await
            .context("task receiver channel closed")?;

        if let Err(res) = result_sender
            .send(OneshotResult {
                cached_task: cached_task.clone(),
                result: compute_cached_proof(
                    cached_task.args,
                    cached_task.rollup_config,
                    cached_task.disk_kv_store,
                    cached_task.precondition,
                    cached_task.proposal_data_hash,
                    cached_task.stitched_executions,
                    cached_task.derivation_cache,
                    cached_task.derivation_trace,
                    cached_task.stitched_preconditions,
                    cached_task.stitched_boot_info,
                    cached_task.stitched_proofs,
                    cached_task.prove_snark,
                    cached_task.force_attempt,
                    cached_task.seek_proof,
                )
                .await,
            })
            .await
        {
            error!("failed to send task result: {res:?}");
        }
    }
}

/// Send a [Oneshot] task to the prover pool and return once the result arrives
#[allow(clippy::too_many_arguments)]
pub async fn compute_oneshot_task(
    args: ProveArgs,
    rollup_config: SoonRollupConfig,
    disk_kv_store: Option<RWLKeyValueStore>,
    precondition: Precondition,
    proposal_data_hash: B256,
    stitched_executions: Vec<Vec<Execution>>,
    derivation_cache: Option<CachedDriver>,
    derivation_trace: Option<Sender<CachedDriver>>,
    stitched_preconditions: Vec<Precondition>,
    stitched_boot_info: Vec<StitchedBootInfo>,
    stitched_proofs: Vec<Receipt>,
    prove_snark: bool,
    force_attempt: bool,
    seek_proof: bool,
    task_sender: Sender<Oneshot>,
) -> Result<OneshotResultResponse, ProvingError> {
    // create proving task
    let cached_task = CachedTask {
        args,
        rollup_config,
        disk_kv_store,
        precondition,
        proposal_data_hash,
        stitched_executions,
        derivation_cache,
        derivation_trace,
        stitched_preconditions,
        stitched_boot_info,
        stitched_proofs,
        prove_snark,
        force_attempt,
        seek_proof,
    };
    // create oneshot channel
    let oneshot_channel = async_channel::bounded(1);
    // dispatch task to pool
    task_sender
        .send(Oneshot {
            cached_task,
            result_sender: oneshot_channel.0,
        })
        .await
        .expect("Oneshot channel closed");
    // wait for result
    oneshot_channel
        .1
        .recv()
        .await
        .expect("oneshot_channel should never panic")
        .result
}

/// Computes a receipt if it is not cached
#[allow(clippy::too_many_arguments)]
pub async fn compute_fpvm_proof(
    mut args: ProveArgs,
    rollup_config: SoonRollupConfig,
    disk_kv_store: Option<RWLKeyValueStore>,
    mut precondition: Precondition,
    proposal_data_hash: B256,
    derivation_cache: Option<Receiver<CachedDriver>>,
    derivation_trace: Option<Sender<CachedDriver>>,
    stitched_preconditions: Vec<Precondition>,
    stitched_boot_info: Vec<StitchedBootInfo>,
    stitched_proofs: Vec<Receipt>,
    prove_snark: bool,
    task_sender: Sender<Oneshot>,
) -> Result<Option<OneshotResultResponse>, ProvingError> {
    // report transaction count
    if !stitched_boot_info.is_empty() {
        info!("Stitching {} sub-proofs", stitched_boot_info.len());
    }
    if stitched_boot_info.len() != stitched_preconditions.len() {
        warn!(
            "Attempting to stitch {} sub-proofs with {} preconditions",
            stitched_boot_info.len(),
            stitched_preconditions.len()
        );
    }

    //  1. try entire proof
    //      on failure, take execution trace
    //      on success, signal driver trace
    //  2. trim derivation tail
    //  3. try head proof
    //      on failure, report error
    //      on success, signal driver trace
    //  3. compute execution-only proofs
    //  4. stitch tail/execution proofs

    // Wait for the cached driver to be reported before derivation unless it is skipped
    let mut derivation_cache = if !args.proving.skip_derivation_proof {
        match derivation_cache {
            Some(receiver) => {
                let cached_driver = receiver
                    .recv()
                    .await
                    .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;
                let derivation_cache_precondition = B256::new(cached_driver.digest().into());
                info!("Received CachedDriver {derivation_cache_precondition}");
                // add cached driver precondition
                precondition.derivation_cache = derivation_cache_precondition;
                Some(cached_driver)
            }
            None => None,
        }
    } else {
        None
    };

    // Check if we can do execution-only proofs
    let can_stitch_executions = args.proving.max_block_executions > 0;
    // Remove block count constraint if execution stitching is disabled
    if !can_stitch_executions {
        args.proving.max_block_executions = usize::MAX;
    }

    // generate master proof
    info!("Attempting complete proof.");
    let stitching_only = args.kona.agreed_l2_output_root == args.kona.claimed_l2_output_root;
    let complete_proof_result = compute_oneshot_task(
        args.clone(),
        rollup_config.clone(),
        disk_kv_store.clone(),
        precondition,
        proposal_data_hash,
        vec![],
        derivation_cache.clone(),
        derivation_trace, // note: the task sends its driver trace if it starts proving
        stitched_preconditions.clone(),
        stitched_boot_info.clone(),
        stitched_proofs.clone(),
        // pass through snark requirement
        prove_snark,
        // force attempting to compute the proof if it only combines boot infos
        stitching_only,
        // skip seeking a complete proof if not proving derivation
        !args.proving.skip_derivation_proof,
        task_sender.clone(),
    )
    .await;

    // Extract execution and derivation traces when possible on error
    let (executed_blocks, derivation_trace, streamed_witness_size) = match complete_proof_result {
        Err(ProvingError::WitnessSizeError(
            _,
            streamed_witness_size,
            _,
            executed_blocks,
            _,
            derivation_trace,
        )) => (executed_blocks, derivation_trace, streamed_witness_size),
        Err(ProvingError::BlockCountError(_, _, executed_blocks, _, derivation_trace)) => {
            (executed_blocks, derivation_trace, 0)
        }
        Err(ProvingError::NotSeekingProof(_, _, executed_blocks, _, derivation_trace, _)) => {
            (executed_blocks, derivation_trace, 0)
        }
        other_result => return Ok(Some(other_result?)),
    };

    // flatten executed l2 blocks
    let (_, execution_cache) = split_executions(executed_blocks.clone());

    // Sanity check proving task
    if execution_cache.is_empty()
        && args.kona.agreed_l2_output_root != args.kona.claimed_l2_output_root
    {
        return Err(ProvingError::OtherError(anyhow!("Insufficient L1 head.")));
    }

    // Check if we can do tail proofs
    let can_stitch_tail_proofs =
        args.proving.num_tail_blocks > 0 && !args.proving.skip_derivation_proof;
    let mut tail_proof_jobs = vec![];
    if can_stitch_tail_proofs && streamed_witness_size > (args.proving.max_witness_size * 90) / 100
    {
        let mut chain_providers = retry_res_ctx_timeout!(20, args.create_providers().await).await;
        // Fetch earliest l1 block to start from
        let l1_tail_number = {
            let safe_block_info = chain_providers
                .l2
                .l2_block_info_by_number(execution_cache[0].artifacts.block_info.block_info.number)
                .await
                .map_err(|_| {
                    ProvingError::OtherError(anyhow!("Failed to fetch l2 safe block info"))
                })?;
            safe_block_info.l1_origin.number
        }
        .max(
            derivation_cache
                .as_ref()
                .map(|cache| cache.cursor.origin.number)
                .unwrap_or_default(),
        );
        let mut l1_tail = retry_res_ctx_timeout!(chain_providers
            .l1
            .get_block_by_number(l1_tail_number.into())
            .await
            .context("get_block_by_number l1_tail_number")?
            .ok_or_else(|| anyhow!("Failed to fetch l1 tail")))
        .await;
        // Create tail proofs
        info!(
            "Scheduling tail proofs from l1 block {l1_tail_number} (derivation_cache={}).",
            derivation_cache.is_some()
        );
        loop {
            let mut tail_derivation_cache = derivation_cache.clone();
            // Job proving args
            let mut args = args.clone();
            args.proving.max_block_executions = 0;
            let mut job_wit_size = 0;
            let should_continue = loop {
                // move l1 tail forward
                l1_tail = retry_res_ctx_timeout!(chain_providers
                    .l1
                    .get_block_by_number(
                        (l1_tail.header.number + args.proving.num_tail_blocks).into()
                    )
                    .await
                    .context("get_block_by_number l1_tail + num_tail_blocks")?
                    .ok_or_else(|| anyhow!("Failed to fetch l1 tail")))
                .await;
                args.kona.l1_head = l1_tail.header.hash;
                // Driver tracing
                let (derivation_trace, traced_driver) = async_channel::bounded(1);
                // trim l1 tail
                info!("Growing tail to l1 block {}.", l1_tail.header.number);
                let derivation_only_result = compute_oneshot_task(
                    args.clone(),
                    rollup_config.clone(),
                    disk_kv_store.clone(),
                    Precondition {
                        derivation_cache: tail_derivation_cache
                            .as_ref()
                            .map(|c| B256::new(c.digest().into()))
                            .unwrap_or_default(),
                        ..precondition
                    },
                    proposal_data_hash,
                    vec![],
                    tail_derivation_cache.clone(),
                    Some(derivation_trace), // note: the task sends its driver trace if witness size is fine
                    vec![],
                    vec![],
                    vec![],
                    false,
                    false,
                    false,
                    task_sender.clone(),
                )
                .await;
                // handle derivation result
                match derivation_only_result.unwrap_err() {
                    ProvingError::NotSeekingProof(preloaded, streamed, ..) => {
                        job_wit_size += streamed + preloaded;
                        // stop growing tail if witness size is above threshold
                        if job_wit_size > args.proving.max_witness_size {
                            break true;
                        }
                        // capture derivation trace for next iteration of tail growth
                        tail_derivation_cache = Some(
                            traced_driver
                                .recv()
                                .await
                                .expect("Failed to receive tail derivation trace."),
                        );
                        continue;
                    }
                    ProvingError::BlockCountError(..) | ProvingError::WitnessSizeError(..) => {
                        // stop growing tail if a block is derived
                        break false;
                    }
                    err => {
                        // propagate unexpected error up on failure to trigger higher-level division
                        return Err(err);
                    }
                }
            };
            // Schedule a tail proof
            if let Some(derivation_trace) = (job_wit_size > 0)
                .then_some(tail_derivation_cache)
                .flatten()
            {
                // move l1 tail backward for the next iteration to start under
                l1_tail = retry_res_ctx_timeout!(chain_providers
                    .l1
                    .get_block_by_number(
                        l1_tail
                            .header
                            .number
                            .saturating_sub(args.proving.num_tail_blocks)
                            .into()
                    )
                    .await
                    .context("get_block_by_number l1_tail - tail_blocks")?
                    .ok_or_else(|| anyhow!("Failed to fetch l1 tail")))
                .await;
                args.kona.l1_head = l1_tail.header.hash;
                // Queue tail workload
                info!(
                    "Scheduling tail proof for claim height {} at l1 tail {}.",
                    args.kona.claimed_l2_block_number, l1_tail.header.number
                );
                tail_proof_jobs.push((
                    args,
                    derivation_cache,
                    B256::new(derivation_trace.digest().into()),
                ));
                // Update main job
                precondition.derivation_cache = B256::new(derivation_trace.digest().into());
                derivation_cache = Some(derivation_trace);
            }
            // Terminate if a block was derived
            if !should_continue {
                info!(
                    "Terminating tail proof scheduling with {} jobs.",
                    tail_proof_jobs.len()
                );
                break;
            }
        }
    }

    // Clear execution cache if we cannot stitch execution proofs
    let num_executed_blocks = executed_blocks.iter().map(|e| e.len()).sum::<usize>();
    let (executed_blocks, execution_cache) = if can_stitch_executions {
        (executed_blocks, execution_cache)
    } else {
        warn!("Skipping execution stitching.");
        (vec![], vec![])
    };

    // Reevaluate complete provability with stitching unless not proving derivation
    if !args.proving.skip_derivation_proof {
        info!(
            "Reevaluating provability with {} cached executions.",
            execution_cache.len()
        );
        let provability_result = compute_oneshot_task(
            args.clone(),
            rollup_config.clone(),
            disk_kv_store.clone(),
            precondition,
            proposal_data_hash,
            executed_blocks.clone(),
            derivation_cache.clone(),
            derivation_trace, // note: the task sends its driver trace if it succeeds
            stitched_preconditions.clone(),
            stitched_boot_info.clone(),
            stitched_proofs.clone(),
            false,
            false,
            false,
            task_sender.clone(),
        )
        .await;
        // propagate unexpected error up on failure to trigger higher-level division
        let Err(ProvingError::NotSeekingProof(.., derivation_trace_hash)) = provability_result
        else {
            warn!("Could not decompose derivation proof into tail/execution proofs.");
            return Ok(Some(provability_result?));
        };
        info!("Proceeding with execution/tail proof decomposition.");
        // update precondition
        if precondition.derivation_trace.is_zero() {
            precondition.derivation_trace = derivation_trace_hash;
        }
    }

    // dispatch execution proofs
    let execution_result_channel = async_channel::unbounded();
    let mut execution_result_pq = BinaryHeap::new();
    let mut num_execution_proofs = 0;
    if can_stitch_executions {
        let mut next_claim_index = args.proving.max_block_executions.min(execution_cache.len()) - 1;
        let mut agreed_l2_output_root = args.kona.agreed_l2_output_root;
        let mut agreed_l2_block_number = args.kona.agreed_l2_block_number;
        let last_claim_index = execution_cache.len() - 1;
        while agreed_l2_output_root != args.kona.claimed_l2_output_root {
            // Create sub-proof job
            let mut job_args = args.clone();
            job_args.kona.l1_head = B256::ZERO;
            job_args.kona.agreed_l2_output_root = agreed_l2_output_root;
            job_args.kona.agreed_l2_block_number = agreed_l2_block_number;
            job_args.kona.claimed_l2_output_root = execution_cache[next_claim_index].claimed_output;
            job_args.kona.claimed_l2_block_number = execution_cache[next_claim_index]
                .artifacts
                .block_info
                .block_info
                .number;
            // advance pointers
            agreed_l2_output_root = job_args.kona.claimed_l2_output_root;
            agreed_l2_block_number = execution_cache[next_claim_index]
                .artifacts
                .block_info
                .block_info
                .number;
            // queue up job
            num_execution_proofs += 1;
            task_sender
                .send(Oneshot {
                    cached_task: create_cached_execution_task(
                        job_args,
                        rollup_config.clone(),
                        disk_kv_store.clone(),
                        &execution_cache,
                    ),
                    result_sender: execution_result_channel.0.clone(),
                })
                .await
                .expect("task_channel should not be closed");
            // next claim
            if next_claim_index == last_claim_index {
                break;
            }
            next_claim_index = next_claim_index
                .saturating_add(args.proving.max_block_executions)
                .min(last_claim_index);
        }
    }

    // dispatch tail proofs
    let num_tail_proofs = tail_proof_jobs.len();
    let mut tail_proof_receivers = Vec::with_capacity(num_tail_proofs);
    for (args, derivation_cache, derivation_trace) in tail_proof_jobs.into_iter() {
        let (result_sender, result_receiver) = async_channel::unbounded();
        task_sender
            .send(Oneshot {
                cached_task: CachedTask {
                    args,
                    rollup_config: rollup_config.clone(),
                    disk_kv_store: disk_kv_store.clone(),
                    precondition: Precondition {
                        proposal_blobs: precondition.proposal_blobs,
                        execution_trace: B256::ZERO,
                        derivation_cache: derivation_cache
                            .as_ref()
                            .map(|c| B256::new(c.digest().into()))
                            .unwrap_or_default(),
                        derivation_trace,
                    },
                    proposal_data_hash,
                    stitched_executions: vec![],
                    derivation_cache,
                    derivation_trace: None, // we don't need to send the trace anywhere
                    stitched_preconditions: vec![],
                    stitched_boot_info: vec![],
                    stitched_proofs: vec![],
                    prove_snark: false,
                    force_attempt: false,
                    seek_proof: true,
                },
                result_sender,
            })
            .await
            .expect("Oneshot task_sender channel closed.");
        // store result receiver
        tail_proof_receivers.push(result_receiver);
    }

    // process (or skip await) execution-only proving results
    let mut dispatched_execution_proofs = num_execution_proofs;
    while execution_result_pq.len() < num_execution_proofs {
        // Wait for more proving results
        let OneshotResult {
            cached_task,
            result,
        } = execution_result_channel
            .1
            .recv()
            .await
            .expect("result_channel should not be closed");
        let Err(err) = result else {
            execution_result_pq.push(OneshotResult {
                cached_task,
                result,
            });
            continue;
        };
        let executed_blocks = cached_task.stitched_executions[0].clone();
        let agreed_block = executed_blocks[0].artifacts.block_info.block_info.number - 1;
        let num_blocks = cached_task.args.kona.claimed_l2_block_number - agreed_block;
        let forced_attempt = num_blocks == 1;
        // divide or bail out on error
        match err {
            ProvingError::WitnessSizeError(preloaded, streamed, limit, e, ..) => {
                if forced_attempt {
                    error!(
                        "Execution-only proof witness size {} + {} above safety threshold {}.",
                        human_bytes(preloaded as f64),
                        human_bytes(streamed as f64),
                        human_bytes(limit as f64),
                    );
                    return Err(ProvingError::WitnessSizeError(
                        preloaded,
                        streamed,
                        limit,
                        e,
                        Box::new(None),
                        None,
                    ));
                }
                warn!(
                    "Execution-only proof witness size {} + {} above safety threshold {}. Splitting workload.",
                        human_bytes(preloaded as f64),
                        human_bytes(streamed as f64),
                        human_bytes(limit as f64),
                )
            }
            ProvingError::NotAwaitingProof => {
                // require one less proof
                num_execution_proofs -= 1;
                continue;
            }
            ProvingError::OtherError(e) => {
                return Err(ProvingError::OtherError(e));
            }
            _ => unreachable!("Unexpected ProvingError {err:?}"),
        }
        // Require additional proof
        num_execution_proofs += 1;
        dispatched_execution_proofs += 1;
        // Split workload at midpoint (num_blocks > 1)
        let mid_point = agreed_block + num_blocks / 2;
        let mid_exec = executed_blocks
            .iter()
            .find(|e| e.artifacts.block_info.block_info.number == mid_point)
            .expect("Failed to find the midpoint of execution.");
        let mid_output = mid_exec.claimed_output;

        // Lower half workload ends at midpoint (inclusive)
        let mut lower_job_args = cached_task.args.clone();
        lower_job_args.kona.claimed_l2_output_root = mid_output;
        lower_job_args.kona.claimed_l2_block_number = mid_point;
        task_sender
            .send(Oneshot {
                cached_task: create_cached_execution_task(
                    lower_job_args,
                    rollup_config.clone(),
                    disk_kv_store.clone(),
                    &execution_cache,
                ),
                result_sender: execution_result_channel.0.clone(),
            })
            .await
            .expect("task_channel should not be closed");

        // upper half workload starts after midpoint
        let mut upper_job_args = cached_task.args;
        upper_job_args.kona.agreed_l2_output_root = mid_output;
        upper_job_args.kona.agreed_l2_block_number =
            mid_exec.artifacts.block_info.block_info.number;
        task_sender
            .send(Oneshot {
                cached_task: create_cached_execution_task(
                    upper_job_args,
                    rollup_config.clone(),
                    disk_kv_store.clone(),
                    &execution_cache,
                ),
                result_sender: execution_result_channel.0.clone(),
            })
            .await
            .expect("task_channel should not be closed");
    }

    // Return execution proof count without stitching if derivation is not required
    if args.proving.skip_derivation_proof {
        warn!("Skipping stitching {dispatched_execution_proofs} execution proofs with derivation.");
        return Err(ProvingError::SkippingDerivation(
            dispatched_execution_proofs,
        ));
    }

    // Read result_pq for stitched executions and proofs
    let (execution_proofs, stitched_executions): (Vec<_>, Vec<_>) = execution_result_pq
        .into_sorted_vec()
        .into_iter()
        .map(|mut r| {
            (
                r.result.unwrap().0,
                r.cached_task.stitched_executions.pop().unwrap(),
            )
        })
        .unzip();

    // process tail proving results
    let mut tail_preconditions = Vec::with_capacity(num_tail_proofs);
    let mut tail_boot_infos = Vec::with_capacity(num_tail_proofs);
    let mut tail_proofs = Vec::with_capacity(num_tail_proofs);
    // iterate over receivers in reverse to enact backwards stitch
    for receiver in tail_proof_receivers.into_iter().rev() {
        let OneshotResult { result, .. } = receiver
            .recv()
            .await
            .expect("result_channel should not be closed");
        // unpack result
        let (receipt, precondition) = match result {
            Err(err) => {
                if !matches!(err, ProvingError::NotAwaitingProof) {
                    error!("Tail proof error: {err:?}");
                }
                continue;
            }
            Ok(result) => result,
        };
        let stitched_boot = StitchedBootInfo::from(ProofJournal::from(&receipt));
        tail_proofs.push(receipt);
        tail_preconditions.push(precondition);
        tail_boot_infos.push(stitched_boot);
    }

    // Combine execution/tail proofs with derivation proof
    if args.proving.skip_await_proof && dispatched_execution_proofs > 0 {
        warn!("Skipping stitching unawaited execution proofs with derivation proof.");
        return Err(ProvingError::NotAwaitingProof);
    }

    if args.proving.skip_await_proof {
        info!(
            "Dispatching stand-alone head proof for {num_tail_proofs} tails and {num_executed_blocks} L2 blocks."
        );
    } else {
        info!(
            "Stitching {}/{} execution proofs and {}/{} tail proofs for {num_executed_blocks} L2 blocks.",
            execution_proofs.len(),
            stitched_executions.len(),
            tail_proofs.len(),
            num_tail_proofs
        );
    }

    Ok(Some(
        compute_oneshot_task(
            args,
            rollup_config,
            disk_kv_store,
            precondition,
            proposal_data_hash,
            stitched_executions,
            derivation_cache,
            None, // driver trace precondition hash enforced by precondition arg having it
            [tail_preconditions, stitched_preconditions].concat(),
            [tail_boot_infos, stitched_boot_info].concat(),
            [tail_proofs, stitched_proofs, execution_proofs].concat(),
            prove_snark,
            true,
            true,
            task_sender.clone(),
        )
        .await?,
    ))
}

pub fn create_cached_execution_task(
    args: ProveArgs,
    rollup_config: SoonRollupConfig,
    disk_kv_store: Option<RWLKeyValueStore>,
    execution_cache: &[Arc<Execution>],
) -> CachedTask {
    let starting_block = execution_cache
        .iter()
        .find(|e| e.agreed_output == args.kona.agreed_l2_output_root)
        .expect("Failed to find the first execution.")
        .artifacts
        .block_info
        .block_info
        .number
        - 1;
    let num_blocks = args.kona.claimed_l2_block_number - starting_block;
    info!(
        "Processing execution-only job with {} blocks from block {}",
        num_blocks, starting_block
    );
    // Extract executed slice
    let executed_blocks = execution_cache
        .iter()
        .filter(|e| {
            let executed_block_number = e.artifacts.block_info.block_info.number;

            starting_block < executed_block_number
                && executed_block_number <= args.kona.claimed_l2_block_number
        })
        .cloned()
        .collect::<Vec<_>>();
    let precondition =
        Precondition::default().execution(exec_precondition_hash(executed_blocks.as_slice()));

    // Force the proving attempt regardless of witness size if we prove just one block
    let force_attempt = num_blocks == 1;
    let executed_blocks = executed_blocks
        .iter()
        .map(|a| a.as_ref().clone())
        .collect::<Vec<_>>();

    CachedTask {
        args,
        rollup_config,
        disk_kv_store,
        precondition,
        proposal_data_hash: B256::ZERO,
        stitched_executions: vec![executed_blocks],
        derivation_cache: None,
        derivation_trace: None,
        stitched_preconditions: vec![],
        stitched_boot_info: vec![],
        stitched_proofs: vec![],
        prove_snark: false,
        force_attempt,
        seek_proof: true,
    }
}

/// Launches the native kailua-soon-kona client-server pair to compute a [OneshotResultResponse]
#[allow(clippy::too_many_arguments)]
pub async fn compute_cached_proof(
    args: ProveArgs,
    rollup_config: SoonRollupConfig,
    disk_kv_store: Option<RWLKeyValueStore>,
    mut precondition: Precondition,
    proposal_data_hash: B256,
    stitched_executions: Vec<Vec<Execution>>,
    derivation_cache: Option<CachedDriver>,
    mut derivation_trace: Option<Sender<CachedDriver>>,
    stitched_preconditions: Vec<Precondition>,
    stitched_boot_info: Vec<StitchedBootInfo>,
    stitched_proofs: Vec<Receipt>,
    prove_snark: bool,
    force_attempt: bool,
    seek_proof: bool,
) -> Result<OneshotResultResponse, ProvingError> {
    // extract single chain kona config
    let mut boot = BootInfo {
        l1_head: args.kona.l1_head,
        agreed_l2_block_number: args.kona.agreed_l2_block_number,
        agreed_l2_output_root: args.kona.agreed_l2_output_root,
        claimed_l2_output_root: args.kona.claimed_l2_output_root,
        claimed_l2_block_number: args.kona.claimed_l2_block_number,
        chain_id: 0,
        rollup_config: rollup_config.clone(),
    };
    let rollup_config_hash = config_hash(&rollup_config);

    // Print complete boot information
    println!("=== Boot Information ===");
    println!("l1_head: {:?}", boot.l1_head);
    println!("agreed_l2_block_number: {}", boot.agreed_l2_block_number);
    println!("agreed_l2_output_root: {:?}", boot.agreed_l2_output_root);
    println!("claimed_l2_output_root: {:?}", boot.claimed_l2_output_root);
    println!("claimed_l2_block_number: {}", boot.claimed_l2_block_number);
    println!("chain_id: {}", boot.chain_id);
    println!("rollup_config: {:?}", boot.rollup_config);
    println!("rollup_config_hash: {:?}", B256::from(rollup_config_hash));
    println!("========================");
    // Choose image id
    let image_id = args.proving.image_id();

    // Check derivation driver cache if needed
    let driver_file = driver_file_name(image_id, &boot, &precondition);
    let trace_derivation = derivation_trace.is_some() || !precondition.derivation_trace.is_zero();
    // Update boot info
    if let Some(derivation_trace) = try_read_driver(&driver_file).await {
        // Update claim if l1 head insufficient
        let claimed_l2_output_root = *derivation_trace.cursor.l2_safe_head_output_root();
        if claimed_l2_output_root != boot.claimed_l2_output_root {
            let claimed_l2_block_number = derivation_trace.cursor.l2_safe_head().block_info.number;
            info!(
                "Correcting claim {}/{} to {claimed_l2_output_root}/{claimed_l2_block_number}",
                boot.claimed_l2_output_root, boot.claimed_l2_block_number
            );
            boot.claimed_l2_output_root = claimed_l2_output_root;
            boot.claimed_l2_block_number = claimed_l2_block_number;
        }
        // Update derivation trace precondition
        if trace_derivation {
            let derivation_trace_hash = B256::new(derivation_trace.digest().into());
            if precondition.derivation_trace.is_zero() {
                precondition.derivation_trace = derivation_trace_hash;
            } else if precondition.derivation_trace != derivation_trace_hash {
                warn!("Precondition derivation trace hash mismatch. Input: {}, Cached: {derivation_trace_hash}", precondition.derivation_trace);
            }
        }
    }
    // Sanity check initial conditions
    if let Some(derivation_cache) = derivation_cache.as_ref() {
        let agreed_l2_output_root = *derivation_cache.cursor.l2_safe_head_output_root();
        if agreed_l2_output_root.is_zero() {
            warn!(
                "DriverCache {} cursor L2 safe head output root is empty.",
                B256::new(derivation_cache.digest().into())
            );
        } else if agreed_l2_output_root != boot.agreed_l2_output_root {
            error!(
                "DriverCache {} cursor L2 safe head output root {agreed_l2_output_root} does not match BootInfo {}",
                B256::new(derivation_cache.digest().into()),
                boot.agreed_l2_output_root
            );
        }
    }

    // Construct expected journal
    let (boot, proof_journal, mut updated_precondition) = stitch_boot_info::<VecOracle>(
        None, // assume l1 head chain continuity on host side
        boot,
        bytemuck::cast::<[u32; 8], [u8; 32]>(image_id).into(),
        args.proving.payout_recipient_address.unwrap_or_default(),
        precondition,
        stitched_preconditions.clone(),
        stitched_boot_info.clone(),
    )
    .await
    .context("Failed to stitch boot info")
    .map_err(ProvingError::OtherError)?;
    let skip_await_proof = args.proving.skip_await_proof;
    // Skip computation if previously saved to disk
    let mut proof_file = proof_file_name(image_id, &proof_journal);
    if Path::new(&proof_file).try_exists().is_ok_and(identity) && seek_proof {
        info!("Proving skipped. Proof file {proof_file} already exists.");
        // Signal cached trace
        if trace_derivation
            && signal_derivation_trace(
                derivation_trace.clone(),
                try_read_driver(&driver_file).await,
            )
            .await
            .is_some()
        {
            // no need to double-send
            let _ = derivation_trace.take();
        }
        // abort remainder of flow if no proof is to be awaited
        if skip_await_proof {
            return Err(ProvingError::NotAwaitingProof);
        }
    } else {
        if seek_proof {
            info!("Computing uncached proof {proof_file}.");
        } else {
            info!("Running native client.");
        }

        // generate a proof using the kailua client and kona server
        crate::client::native::run_native_client(
            args.clone(),
            disk_kv_store,
            proposal_data_hash,
            stitched_executions,
            derivation_cache,
            trace_derivation,
            derivation_trace,
            stitched_preconditions.clone(),
            stitched_boot_info.clone(),
            stitched_proofs,
            prove_snark,
            force_attempt,
            seek_proof,
        )
        .await?;
    }

    // Load cached driver if tracing derivation is required
    let derivation_trace = if trace_derivation {
        try_read_driver(&driver_file).await
    } else {
        None
    };
    // Correct precondition and target proof file if needed
    if trace_derivation {
        if let Some(derivation_trace) = derivation_trace.as_ref() {
            // Update derivation trace precondition
            precondition.derivation_trace = B256::new(derivation_trace.digest().into());
            // Recalculate receipt file name with new precondition derivation trace
            let claimed_l2_output_root = *derivation_trace.cursor.l2_safe_head_output_root();
            let (_, proof_journal, precondition) = stitch_boot_info::<VecOracle>(
                None, // assume l1 head chain continuity on host side
                BootInfo {
                    // update l2 claim if l1 head insufficient
                    claimed_l2_output_root: if claimed_l2_output_root.is_zero() {
                        boot.agreed_l2_output_root
                    } else {
                        claimed_l2_output_root
                    },
                    claimed_l2_block_number: derivation_trace
                        .cursor
                        .l2_safe_head()
                        .block_info
                        .number,
                    ..boot
                },
                bytemuck::cast::<[u32; 8], [u8; 32]>(image_id).into(),
                args.proving.payout_recipient_address.unwrap_or_default(),
                precondition,
                stitched_preconditions,
                stitched_boot_info,
            )
            .await
            .context("Failed to stitch boot info.")
            .map_err(ProvingError::OtherError)?;
            proof_file = proof_file_name(image_id, &proof_journal);
            updated_precondition = precondition;
        } else {
            error!("Missing expected derivation trace {driver_file}.");
        }
    }
    // Load receipt
    let receipt = read_bincoded_file(&proof_file)
        .await
        .context(format!("Failed to read proof file {proof_file} contents."))
        .map_err(|e| ProvingError::OtherError(anyhow!(e)))?;
    // Return combined response
    Ok((receipt, updated_precondition))
}
