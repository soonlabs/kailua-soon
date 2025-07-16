use crate::{executor::Execution, oracle::WitnessOracle, test::mock::MockOracle};
use alloy_consensus::{Header, Sealed};
use alloy_eips::eip7685::Requests;
use alloy_evm::block::BlockExecutionResult;
use alloy_primitives::{Address, Bytes, B256};
use anyhow::Result;
use bridge::pda::{spl_token_mint_pubkey, spl_token_owner_pubkey};
use crossbeam_channel::Receiver;
use fraud_executor::accounts::{AccountPairs, SoonAccounts};
use fraud_executor::outcome::BlockBuildingOutcome;
use kona_executor::{cal_extra_accounts_hash, cal_init_accounts_hash, cal_init_state_root_hash};
use kona_preimage::PreimageKey;
use kona_proof::BootInfo;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use solana_sdk::{
    account::ReadableAccount, program_pack::Pack, pubkey::Pubkey, signature::Keypair,
    signer::Signer, transaction::VersionedTransaction,
};
use soon_node::derive::driver::L2ChainProviderImmutable;
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
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    rollup_config::SoonRollupConfig,
};
use spl_token::state::Mint;
use std::collections::HashMap;
use std::{
    env::{current_dir, var},
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::info;

const L1_NUMBER: u64 = 100;

#[derive(Debug, Default, Clone)]
pub struct OracleStorageItems {
    pub safe_head: L2BlockInfo,
    pub init_accounts: HashMap<u64, SoonAccounts>,
    pub sysvar_accounts: HashMap<u64, AccountPairs>,
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

pub(crate) fn soon_to_execution_cache() -> Result<(BootInfo, Vec<Arc<Execution>>, MockOracle)> {
    let temp = tempfile::tempdir()?;
    let (mut producer, identity, metadata, complete_receiver) = new_soon(temp.path())?;

    let (boot_info, executions, oracle_storage_items) =
        blocks_to_execution_cache(&mut producer, &identity, &metadata, complete_receiver)?;
    let mut oracle = MockOracle::new(boot_info.clone());
    save_to_oracle(&mut oracle, &boot_info, &oracle_storage_items)?;
    Ok((boot_info, executions, oracle))
}

fn current_executor_state_root(executor: &SharedExecutor) -> Result<B256> {
    let state_root = executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        Ok(soon_accounts.state_root())
    })?;
    Ok(state_root)
}

fn save_to_oracle(
    oracle: &mut MockOracle,
    boot_info: &BootInfo,
    storage_items: &OracleStorageItems,
) -> Result<()> {
    info!("save to oracle details: {:?}", storage_items);

    // save safe head
    let mut agreed_output_data = [0u8; 128];
    agreed_output_data[96..].copy_from_slice(&storage_items.safe_head.block_info.hash[..]);
    oracle.insert_preimage(
        PreimageKey::new_keccak256(boot_info.agreed_l2_output_root.0),
        agreed_output_data.to_vec(),
    );
    oracle.insert_preimage(
        PreimageKey::new_keccak256(storage_items.safe_head.block_info.hash.0),
        bincode::serialize(&storage_items.safe_head)?,
    );

    // save init accounts
    storage_items
        .init_accounts
        .iter()
        .for_each(|(slot, accounts)| {
            oracle.insert_preimage(
                PreimageKey::new_keccak256(cal_init_accounts_hash(*slot).0),
                bincode::serialize(accounts).unwrap(),
            );
            oracle.insert_preimage(
                PreimageKey::new_keccak256(cal_init_state_root_hash(*slot).0),
                accounts.state_root().to_vec(),
            )
        });

    // save sysvar accounts
    for (slot, account_pairs) in &storage_items.sysvar_accounts {
        oracle.insert_preimage(
            PreimageKey::new_keccak256(cal_extra_accounts_hash(*slot).0),
            bincode::serialize(account_pairs)?,
        );
    }

    Ok(())
}

pub(crate) fn blocks_to_execution_cache(
    producer: &mut Producer<SharedExecutor, MockInstant>,
    identity: &Keypair,
    metadata: &TokenMetadata,
    complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
) -> Result<(BootInfo, Vec<Arc<Execution>>, OracleStorageItems)> {
    let mut executions = Vec::new();
    let mut boot_info = BootInfo {
        l1_head: B256::ZERO,
        agreed_l2_output_root: B256::ZERO,
        claimed_l2_output_root: B256::ZERO,
        claimed_l2_block_number: 0,
        chain_id: 0,
        rollup_config: SoonRollupConfig::default(),
    };
    let mut storage_items = OracleStorageItems::default();
    let executor = producer.get_executor().clone();
    executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        storage_items
            .init_accounts
            .insert(s.current_slot(), soon_accounts);
        Ok(())
    })?;
    let agreed_output = current_executor_state_root(&executor)?;
    let slot_1_head = executor.l2_block_info_by_number_immut(executor.latest_slot()?)?;
    storage_items.safe_head = slot_1_head;
    info!("storage safe head: {:?}", storage_items.safe_head);
    boot_info.agreed_l2_output_root = agreed_output;

    // === slot 1
    let derive_block_1 = new_derive_block(metadata.to.pubkey(), L1_NUMBER);
    let raw = RawBlock::try_init(derive_block_1, 0, &Default::default())?;
    producer.mine_with_block(Some(raw.clone()))?;
    complete_receiver.try_recv()?;
    // assert l1 block info state
    assert_eq!(executor.latest_slot()?, 1);

    let claimed_output = current_executor_state_root(&executor)?;
    info!("soon slot 1 state root: {:?}", claimed_output);
    executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        storage_items
            .init_accounts
            .insert(s.current_slot(), soon_accounts);
        storage_items
            .sysvar_accounts
            .insert(s.current_slot(), s.export_sysvars()?);
        Ok(())
    })?;

    let slot_1_head = executor.l2_block_info_by_number_immut(executor.latest_slot()?)?;
    let execution = to_execution(raw, agreed_output, claimed_output, slot_1_head)?;
    executions.push(Arc::new(execution));

    // === slot 2
    let agreed_output = claimed_output;
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
    info!("soon slot 2 state root: {:?}", claimed_output);
    executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        storage_items
            .init_accounts
            .insert(s.current_slot(), soon_accounts);
        storage_items
            .sysvar_accounts
            .insert(s.current_slot(), s.export_sysvars()?);
        Ok(())
    })?;

    let slot_2_head = executor.l2_block_info_by_number_immut(executor.latest_slot()?)?;
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
        slot_2_head,
    )?;
    executions.push(Arc::new(execution));

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
    info!("soon slot 3 state root: {:?}", claimed_output);
    boot_info.claimed_l2_output_root = claimed_output;
    boot_info.claimed_l2_block_number = producer.get_executor().latest_slot()?;
    info!("boot info: {:?}", boot_info);
    executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        storage_items
            .init_accounts
            .insert(s.current_slot(), soon_accounts);
        storage_items
            .sysvar_accounts
            .insert(s.current_slot(), s.export_sysvars()?);
        Ok(())
    })?;

    let slot_3_head = executor.l2_block_info_by_number_immut(executor.latest_slot()?)?;
    let execution = to_execution(raw, agreed_output, claimed_output, slot_3_head)?;
    executions.push(Arc::new(execution));

    Ok((boot_info, executions, storage_items))
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
            var("CARGO_MANIFEST_DIR")
                .ok()
                .map_or_else(|| current_dir().ok(), |s| Some(PathBuf::from(s)))
                .unwrap()
                .join("../../../soon/node/programs/target/deploy"),
        ),
    )?;

    let (producer, _, complete_receiver) = new_producer(path, identity.clone())?;

    let metadata = TokenMetadata::default();
    Ok((producer, identity, metadata, complete_receiver))
}
