use crate::executor::Execution;
use alloy_consensus::{Header, Sealed};
use alloy_eips::eip7685::Requests;
use alloy_evm::block::BlockExecutionResult;
use alloy_primitives::{Address, Bytes, B256};
use anyhow::Result;
use bridge::pda::{spl_token_mint_pubkey, spl_token_owner_pubkey};
use crossbeam_channel::Receiver;
use fraud_executor::accounts::SoonAccounts;
use kona_executor::BlockBuildingOutcome;
use kona_proof::BootInfo;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use solana_sdk::{
    account::ReadableAccount, program_pack::Pack, pubkey::Pubkey, signature::Keypair,
    signer::Signer, transaction::VersionedTransaction,
};
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::{
        producer::Producer,
        tests::{
            create_derived_deposit_erc20_tx, create_spl_tx, init_soon_genesis,
            new_attributed_deposit_tx, new_derive_block, new_producer, DEPOSIT_AMOUNT,
        },
    },
};
use soon_primitives::blocks::{BlockInfo, L2BlockInfo, RawBlock};
use spl_token::state::Mint;
use std::{path::Path, sync::Arc};

const L1_NUMBER: u64 = 100;

pub(crate) fn soon_to_execution_cache() -> Result<(BootInfo, Vec<Arc<Execution>>)> {
    let temp = tempfile::tempdir()?;
    let (mut producer, identity, metadata, complete_receiver) = new_soon(temp.path())?;

    let executions =
        blocks_to_execution_cache(&mut producer, &identity, &metadata, complete_receiver)?;

    Ok((
        BootInfo {
            l1_head: B256::ZERO,
            agreed_l2_output_root: B256::ZERO,
            claimed_l2_output_root: B256::ZERO,
            claimed_l2_block_number: 0,
            chain_id: 0,
            rollup_config: Default::default(),
        },
        executions,
    ))
}

fn current_executor_state_root(executor: &SharedExecutor) -> Result<B256> {
    let state_root = executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        Ok(soon_accounts.state_root())
    })?;
    Ok(state_root)
}

pub(crate) fn blocks_to_execution_cache(
    producer: &mut Producer<SharedExecutor, MockInstant>,
    identity: &Keypair,
    metadata: &TokenMetadata,
    complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
) -> Result<Vec<Arc<Execution>>> {
    let mut result = Vec::new();
    let executor = producer.get_executor().clone();

    // === slot 2
    let agreed_output = current_executor_state_root(&executor)?;
    // append a `CreateSPL` tx into the block
    let last_blockhash = executor.storage_query(|s| Ok(s.current_bank().last_blockhash()))?;
    let create_spl_tx = create_spl_tx(
        metadata.remote_token,
        identity,
        identity,
        last_blockhash,
        &metadata.token_name,
        &metadata.token_symbol,
        &metadata.uri,
    )?;
    producer.add_tx(create_spl_tx.clone())?;
    producer.mine_with_block(None)?;
    complete_receiver.try_recv()?;
    // assert deposit ETH state
    let to_account_data = executor.get_account_by_slot(2, &metadata.to.pubkey())?;
    assert_eq!(to_account_data.lamports(), DEPOSIT_AMOUNT);
    // assert create spl token state
    let spl_token_mint_account = executor.get_account_by_slot(
        2,
        &spl_token_mint_pubkey(&metadata.remote_token.0 .0.into()),
    )?;
    let mint = Mint::unpack(spl_token_mint_account.data())?;
    assert_eq!(
        mint.mint_authority.unwrap(),
        spl_token_owner_pubkey(&metadata.remote_token.0 .0.into())
    );
    let claimed_output = current_executor_state_root(&executor)?;

    let l1_info_tx = new_attributed_deposit_tx(L1_NUMBER, 1);
    let execution = tx_to_execution(
        vec![
            create_spl_tx.into(),
            l1_info_tx
                .to_sanitized_transaction(&Default::default())?
                .to_versioned_transaction(),
        ],
        agreed_output,
        claimed_output,
    )?;
    result.push(Arc::new(execution));

    // === slot 3
    let agreed_output = claimed_output;
    let mut derive_block_2 = new_derive_block(metadata.to.pubkey(), L1_NUMBER + 1);
    // deposit erc20 token
    derive_block_2
        .deposit_txs
        .push(create_derived_deposit_erc20_tx(
            metadata.remote_token,
            metadata.to.pubkey(),
        ));
    let raw = RawBlock::try_init(derive_block_2, 0, &Default::default())?;
    producer.mine_with_block(Some(raw.clone()))?;
    complete_receiver.try_recv()?;
    let claimed_output = current_executor_state_root(&executor)?;

    let execution = to_execution(raw, agreed_output, claimed_output)?;
    result.push(Arc::new(execution));

    Ok(result)
}

pub(crate) fn tx_to_execution(
    txs: Vec<VersionedTransaction>,
    agreed_output: B256,
    claimed_output: B256,
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
            header: Sealed::new(Header::default()),
            execution_result: BlockExecutionResult {
                receipts: vec![],
                requests: Requests::new(vec![]),
                gas_used: 0,
            },
        },
        claimed_output,
    })
}

pub(crate) fn to_execution(
    block: RawBlock,
    agreed_output: B256,
    claimed_output: B256,
) -> Result<Execution> {
    let txs = block
        .transactions
        .iter()
        .map(|tx| tx.to_versioned_transaction())
        .collect::<Vec<_>>();
    tx_to_execution(txs, agreed_output, claimed_output)
}

pub(crate) fn encode_tx(tx: VersionedTransaction) -> Result<Bytes> {
    let tx_bytes = bincode::serialize(&tx)?;
    Ok(Bytes::from(tx_bytes))
}

pub(crate) fn new_soon(
    path: &Path,
) -> Result<(
    Producer<SharedExecutor, MockInstant>,
    Arc<Keypair>,
    TokenMetadata,
    Receiver<(L2BlockInfo, Option<BlockInfo>)>,
)> {
    let identity = Arc::new(Keypair::new());
    init_soon_genesis(path, &identity, true)?;

    let (mut producer, _, complete_receiver) = new_producer(path, identity.clone())?;
    let executor = producer.get_executor().clone();

    let metadata = TokenMetadata::default();

    // === slot 1
    let derive_block_1 = new_derive_block(metadata.to.pubkey(), L1_NUMBER);
    let raw = RawBlock::try_init(derive_block_1, 0, &Default::default())?;
    producer.mine_with_block(Some(raw.clone()))?;
    complete_receiver.try_recv()?;
    // assert l1 block info state
    assert_eq!(executor.latest_slot()?, 1);

    Ok((producer, identity, metadata, complete_receiver))
}

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
