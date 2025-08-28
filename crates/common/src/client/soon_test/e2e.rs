use super::{
    fetch_info_and_update_execution_storage_items, new_soon, ExecutionStorageItems, TokenMetadata,
};
use crate::client::soon_test::{to_execution, L1_NUMBER};
use crate::{executor::Execution, oracle::WitnessOracle, test::mock::MockOracle};
use alloy_primitives::{keccak256, B256};
use alloy_rlp::{BytesMut, Encodable};
use anyhow::Result;
use bridge::pda::{spl_token_mint_pubkey, spl_token_owner_pubkey};
use crossbeam_channel::Receiver;
use kona_executor::{
    cal_init_state_root_hash, cal_soon_accounts_hash, cal_svm_clock_timestamp, cal_svm_leader,
    cal_svm_parent_info,
};
use kona_preimage::PreimageKey;
use kona_proof::BootInfo;
use rkyv::ser::sharing::Share;
use solana_sdk::{
    account::ReadableAccount, program_pack::Pack, signature::Keypair, signer::Signer,
};
use soon_node::node::tests::{new_derive_block_with_mock_l1, MockEthL1Node};
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::mpt::MptRunner,
    node::{producer::Producer, tests::create_spl_tx},
};
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    rollup_config::SoonRollupConfig,
};
use spl_token::state::Mint;
use std::sync::Arc;
use tracing::info;

pub type E2eSoonProducer = Producer<SharedExecutor, MockInstant>;

/// An all-in-one environment holding every component for e2e fraud proof testing between
/// Soon Execution and Kona Execution.
pub struct E2eKailuaSoonEnvironment {
    pub e2e_producer: E2eSoonProducer,
    pub mpt_runner: MptRunner,
    pub identity: Arc<Keypair>,
    pub metadata: TokenMetadata,
    pub complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
    pub l1_node: MockEthL1Node,
}

pub async fn init_soon_env(relative_to_soon: Option<&str>) -> Result<E2eKailuaSoonEnvironment> {
    // init soon producer.
    let mut mock_l1_node = MockEthL1Node::new(L1_NUMBER, 12);
    let temp = tempfile::tempdir()?;
    let (mut e2e_producer, identity, metadata, complete_receiver) =
        new_soon(temp.path(), relative_to_soon, &mut mock_l1_node)?;

    // init mpt calculation.
    let mpt_path = tempfile::tempdir()?;
    let exit = e2e_producer
        .get_executor()
        .storage_query(|storage| Ok(storage.exit.clone()))?;

    let (_, r, s, _) = e2e_producer
        .get_executor()
        .storage_query(|storage| Ok(storage.signal_hub.mpt_update_chanel.clone().unwrap()))?;

    let mpt_runner = MptRunner::new(mpt_path.path(), r, s, exit)?;

    Ok(E2eKailuaSoonEnvironment {
        e2e_producer,
        mpt_runner,
        identity,
        metadata,
        complete_receiver,
        l1_node: mock_l1_node,
    })
}

/// promote_multi_tx will promote more than 200 random blocks
/// including modify `data` and `lamports` of all accounts.
pub async fn promote_multi_tx(
    env: &mut E2eKailuaSoonEnvironment,
) -> Result<(BootInfo, Vec<Arc<Execution>>, ExecutionStorageItems)> {
    let blocks = 300;
    let mut accounts = Vec::new();
    let mut spl_accounts = Vec::new();

    // on slot 2, generate multi random accounts and spl ata.
    // need to airdrop enough lamports + spl token
    for _ in 0..50 {
        let account = solana_sdk::signature::Keypair::new();
        accounts.push(account);
    }
}
