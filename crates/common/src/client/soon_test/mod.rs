use crate::executor::Execution;
use alloy_consensus::Header;
use alloy_primitives::{Address, Bytes, B256};
use anyhow::Result;
use crossbeam_channel::Receiver;
use fraud_executor::{
    accounts::{AccountPairs, SoonAccounts},
    outcome::BlockBuildingOutcome,
};
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::VersionedTransaction};
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::{
        producer::Producer,
        tests::{init_soon_genesis, new_derive_block, new_producer},
    },
};
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    l2blocks::L2Block,
};
use std::{collections::HashMap, path::Path, sync::Arc};

pub(crate) mod derivation;
pub(crate) mod execution;

#[allow(unused_imports)]
pub(crate) use derivation::soon_to_derivation;
#[allow(unused_imports)]
pub(crate) use execution::soon_to_execution_cache;

#[derive(Debug, Default, Clone)]
pub struct ExecutionStorageItems {
    pub safe_head: L2BlockInfo,
    pub l2_blocks: HashMap<u64, L2Block>,
    pub init_accounts: HashMap<u64, SoonAccounts>,
    pub sysvar_accounts: HashMap<u64, AccountPairs>,
    pub slot_hash_pairs: HashMap<u64, (B256, B256)>,
}

#[derive(Debug, Default, Clone)]
pub struct DerivationStorageItems {
    pub execution: ExecutionStorageItems,
    pub l1_heads: HashMap<B256, Header>,
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
            remote_token: Address::random(),
            to: Keypair::new(),
            token_name: "Test".to_string(),
            token_symbol: "TST".to_string(),
            uri: "https://ipfs.io/ipfs/QmXRVXSRbH9nKYPgVfakXRhDhEaXWs6QYu3rToadXhtHPr".to_string(),
        }
    }
}

pub(crate) fn tx_to_execution(
    txs: Vec<VersionedTransaction>,
    agreed_output: B256,
    claimed_output: B256,
    header: L2BlockInfo,
) -> Result<Execution> {
    let txs = txs
        .into_iter()
        .map(encode_tx)
        .collect::<Result<Vec<Bytes>>>()?;
    Ok(Execution {
        agreed_output,
        attributes: OpPayloadAttributes {
            transactions: Some(txs),
            ..Default::default()
        },
        artifacts: BlockBuildingOutcome {
            header,
            execution_result: vec![],
        },
        claimed_output,
    })
}

pub(crate) fn to_execution(
    block: RawBlock,
    agreed_output: B256,
    claimed_output: B256,
    header: L2BlockInfo,
) -> Result<Execution> {
    let txs = block
        .transactions
        .iter()
        .map(|tx| tx.to_versioned_transaction())
        .collect::<Vec<_>>();
    tx_to_execution(txs, agreed_output, claimed_output, header)
}

pub(crate) fn encode_tx(tx: VersionedTransaction) -> Result<Bytes> {
    let tx_bytes = bincode::serialize(&tx)?;
    Ok(Bytes::from(tx_bytes))
}

#[allow(clippy::type_complexity)]
pub(crate) fn new_soon(
    path: &Path,
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
                .join("../../../soon/node/programs/target/deploy"),
        ),
    )?;

    let (mut producer, _, complete_receiver) = new_producer(path, identity.clone())?;
    let metadata = TokenMetadata::default();

    // === slot 1
    let derive_block_1 = new_derive_block(metadata.to.pubkey(), L1_NUMBER);
    let raw = RawBlock::try_init(derive_block_1, 0, &Default::default())?;
    producer.mine_with_block(Some(raw.clone()))?;
    complete_receiver.try_recv()?;
    // assert l1 block info state
    assert_eq!(producer.get_executor().latest_slot()?, 1);

    Ok((producer, identity, metadata, complete_receiver))
}

pub(crate) fn current_executor_state_root(executor: &SharedExecutor) -> Result<B256> {
    let state_root = executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        Ok(soon_accounts.state_root())
    })?;
    Ok(state_root)
}
