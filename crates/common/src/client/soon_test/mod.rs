use crate::{
    client::core::{recover_collected_executions, run_core_client_ex},
    executor::Execution,
};
use alloy_consensus::Header;
use alloy_primitives::{Address, Bytes, B256};
use anyhow::{Context, Result};
use bridge::solana_program::fee_calculator::FeeRateGovernor;
use crossbeam_channel::Receiver;
use fraud_executor::{accounts::SoonAccounts, outcome::BlockBuildingOutcome};
use kona_executor::L2BlockBuilder;
use kona_preimage::CommsClient;
use kona_proof::{BootInfo, FlushableCache};
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use solana_sdk::hash::Hash;
use solana_sdk::{signature::Keypair, signer::Signer};
use soon_derive::prelude::L2ChainProvider;
use soon_derive::traits::BlobProvider;
use soon_node::derive::driver::L2ChainProviderImmutable;
use soon_node::node::tests::{new_derive_block_with_mock_l1, MockEthL1Node};
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::{
        producer::Producer,
        tests::{init_soon_genesis, new_producer},
    },
};
use soon_primitives::blocks::L1Transaction;
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    l2blocks::L2Block,
};
use std::fmt::Debug;
use std::sync::Mutex;
use std::{collections::HashMap, path::Path, sync::Arc};

pub(crate) mod derivation;
pub(crate) mod execution;
pub(crate) mod providers;

#[allow(unused_imports)]
pub use derivation::soon_to_derivation;
#[allow(unused_imports)]
pub use execution::soon_to_execution_cache;
#[allow(unused_imports)]
pub(crate) use providers::{TestDaProvider, TestOracleL1ChainProvider, TestOracleL2ChainProvider};

#[derive(Debug, Default, Clone)]
pub struct ExecutionStorageItems {
    pub safe_head: L2BlockInfo,
    pub l2_blocks: HashMap<u64, L2Block>,
    pub soon_accounts: HashMap<u64, SoonAccounts>,
    pub clock_timestamps: HashMap<u64, i64>,
    pub bank_hashes: HashMap<u64, Hash>,
}

#[derive(Debug, Default, Clone)]
pub struct DerivationStorageItems {
    pub execution: ExecutionStorageItems,
    pub l1_heads: HashMap<B256, Header>,
    pub l1_transactions: HashMap<B256, Vec<L1Transaction>>,
    pub da_data: HashMap<B256, Vec<u8>>,
}

const L1_NUMBER: u64 = 100;

pub(crate) struct TokenMetadata {
    pub remote_token: Address,
    pub to: Keypair,
    pub token_name: String,
    pub token_symbol: String,
    pub uri: String,
}

impl Default for TokenMetadata {
    fn default() -> Self {
        Self {
            remote_token: Address::ZERO,
            to: Keypair::new(),
            token_name: "Test".to_string(),
            token_symbol: "TST".to_string(),
            uri: "https://ipfs.io/ipfs/QmXRVXSRbH9nKYPgVfakXRhDhEaXWs6QYu3rToadXhtHPr".to_string(),
        }
    }
}

pub fn derive_to_execution<
    E,
    O: CommsClient + FlushableCache + Send + Sync + Debug,
    B: BlobProvider + Send + Sync + Debug + Clone,
>(
    boot_info: BootInfo,
    oracle: Arc<O>,
    blob_provider: B,
    precondition_validation_data_hash: B256,
    expected_precondition_hash: B256,
) -> Result<Vec<Arc<Execution>>>
where
    <B as BlobProvider>::Error: Debug,
    E: L2BlockBuilder<TestOracleL2ChainProvider<O>, TestOracleL2ChainProvider<O>>
        + Send
        + Sync
        + Debug,
{
    let clone_oracle = oracle.clone();
    let (l1_provider, l2_provider, da_provider) =
        kona_proof::block_on(async move { initialize_test_providers(clone_oracle).await })?;
    let collection_target = Arc::new(Mutex::new(Vec::new()));
    let (result_boot_info, precondition_hash) = run_core_client_ex::<
        E,
        O,
        B,
        TestOracleL1ChainProvider<O>,
        TestOracleL2ChainProvider<O>,
        TestDaProvider<O>,
    >(
        precondition_validation_data_hash,
        oracle.clone(),
        blob_provider,
        l1_provider,
        l2_provider,
        da_provider,
        vec![],
        Some(collection_target.clone()),
    )
    .context("run_core_client")?;

    assert_eq!(result_boot_info.l1_head, boot_info.l1_head);
    assert_eq!(
        result_boot_info.agreed_l2_output_root,
        boot_info.agreed_l2_output_root
    );
    assert_eq!(
        result_boot_info.claimed_l2_output_root,
        boot_info.claimed_l2_output_root
    );
    assert_eq!(
        result_boot_info.claimed_l2_block_number,
        boot_info.claimed_l2_block_number
    );
    assert_eq!(result_boot_info.chain_id, boot_info.chain_id);

    assert_eq!(expected_precondition_hash, precondition_hash);

    let execution_cache =
        recover_collected_executions(collection_target, boot_info.claimed_l2_output_root);

    Ok(execution_cache)
}

pub(crate) fn to_execution(
    block: L2Block,
    agreed_output: B256,
    claimed_output: B256,
    header: L2BlockInfo,
) -> Result<Execution> {
    Ok(Execution {
        agreed_output,
        attributes: l2_block_to_op_attributes(block)?,
        artifacts: BlockBuildingOutcome {
            block_info: header,
            state_root: claimed_output,
            withdraw_root: B256::ZERO,
            execution_result: vec![],
            fee_rate_governor: FeeRateGovernor::default(),
            signature_count: 0,
        },
        claimed_output,
    })
}

fn l2_block_to_op_attributes(block: L2Block) -> Result<OpPayloadAttributes> {
    Ok(OpPayloadAttributes {
        transactions: Some(
            block
                .transactions
                .into_iter()
                .map(|tx| {
                    let tx_bytes = bincode::serialize(&tx)?;
                    Ok(Bytes::from(tx_bytes))
                })
                .collect::<Result<_>>()?,
        ),
        ..Default::default()
    })
}

#[allow(clippy::type_complexity)]
pub(crate) fn new_soon(
    path: &Path,
    relative_to_soon: Option<&str>,
    l1_node: &mut MockEthL1Node,
) -> Result<(
    Producer<SharedExecutor, MockInstant>,
    Arc<Keypair>,
    TokenMetadata,
    Receiver<(L2BlockInfo, Option<BlockInfo>)>,
)> {
    let identity = Arc::new(Keypair::new());
    init_soon_genesis(
        path,
        &identity,
        true,
        Some(
            std::env::var("CARGO_MANIFEST_DIR")
                .ok()
                .map_or_else(
                    || std::env::current_dir().ok(),
                    |s| Some(std::path::PathBuf::from(s)),
                )
                .unwrap()
                .join(relative_to_soon.unwrap_or("../../.."))
                .join("soon/node/programs/target/deploy"),
        ),
    )?;

    let (mut producer, _, complete_receiver) = new_producer(path, identity.clone())?;
    let metadata = TokenMetadata::default();

    // === slot 1
    let derive_block_1 = new_derive_block_with_mock_l1(l1_node, metadata.to.pubkey());
    let raw = RawBlock::try_init(derive_block_1, 0, &Default::default())?;
    producer.mine_with_block(Some(raw.clone()))?;
    complete_receiver.try_recv()?;
    // assert l1 block info state
    assert_eq!(producer.get_executor().latest_slot()?, 1);

    Ok((producer, identity, metadata, complete_receiver))
}

pub(crate) async fn fetch_info_and_update_execution_storage_items(
    executor: &mut SharedExecutor,
    storage_items: &mut ExecutionStorageItems,
) -> Result<(L2BlockInfo, B256, L2Block)> {
    let slot = executor.latest_slot()?;
    let l2_block = executor.block_by_number(slot).await?;
    storage_items.l2_blocks.insert(slot, l2_block.clone());

    let state_root = executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        let state_root = soon_accounts.state_root();
        storage_items.soon_accounts.insert(slot, soon_accounts);
        storage_items
            .bank_hashes
            .insert(slot, s.current_bank().hash());
        storage_items
            .clock_timestamps
            .insert(slot, s.current_bank().clock().unix_timestamp);
        Ok(state_root)
    })?;

    let head = executor.l2_block_info_by_number_immut(slot)?;

    Ok((head, state_root, l2_block))
}

pub(crate) async fn initialize_test_providers<O>(
    oracle: Arc<O>,
) -> Result<(
    TestOracleL1ChainProvider<O>,
    TestOracleL2ChainProvider<O>,
    TestDaProvider<O>,
)>
where
    O: CommsClient + FlushableCache + Send + Sync + Debug,
{
    let l1_provider = TestOracleL1ChainProvider::new(oracle.clone());
    let l2_provider = TestOracleL2ChainProvider::new(oracle.clone());
    let da_provider = TestDaProvider::new(oracle);

    Ok((l1_provider, l2_provider, da_provider))
}
