use super::{new_soon, ExecutionStorageItems, TokenMetadata};
use crate::client::soon_test::{
    current_executor_state_root, to_execution, tx_to_execution, L1_NUMBER,
};
use crate::{executor::Execution, oracle::WitnessOracle, test::mock::MockOracle};
use alloy_primitives::{keccak256, B256};
use alloy_rlp::Encodable;
use anyhow::Result;
use bridge::pda::{spl_token_mint_pubkey, spl_token_owner_pubkey};
use crossbeam_channel::Receiver;
use fraud_executor::accounts::SoonAccounts;
use kona_executor::{
    cal_extra_accounts_hash, cal_init_accounts_hash, cal_init_state_root_hash, slot_hash_pair_hash,
};
use kona_preimage::PreimageKey;
use kona_proof::BootInfo;
use solana_sdk::{
    account::ReadableAccount, program_pack::Pack, signature::Keypair, signer::Signer,
};
use soon_derive::traits::L2ChainProvider;
use soon_node::derive::driver::L2ChainProviderImmutable;
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::{
        producer::Producer,
        tests::{
            create_derived_deposit_erc20_tx, create_spl_tx, new_attributed_deposit_tx,
            new_derive_block, DEPOSIT_AMOUNT,
        },
    },
};
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    rollup_config::SoonRollupConfig,
};
use spl_token::state::Mint;
use std::sync::Arc;
use tracing::info;

#[allow(dead_code)]
pub async fn soon_to_execution_cache(
    relative_to_soon: Option<&str>,
) -> Result<(BootInfo, MockOracle)> {
    let temp = tempfile::tempdir()?;
    let (mut producer, identity, metadata, complete_receiver) =
        new_soon(temp.path(), relative_to_soon)?;

    let (boot_info, executions, oracle_storage_items) =
        blocks_to_execution_cache(&mut producer, &identity, &metadata, complete_receiver).await?;
    let mut oracle = MockOracle::new_with_executions(boot_info.clone(), executions);
    executions_save_to_oracle(&mut oracle, &boot_info, &oracle_storage_items)?;
    Ok((boot_info, oracle))
}

pub(crate) fn executions_save_to_oracle(
    oracle: &mut MockOracle,
    boot_info: &BootInfo,
    storage_items: &ExecutionStorageItems,
) -> Result<()> {
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

    // save slot hash pairs
    for (slot, (hash, parent_hash)) in &storage_items.slot_hash_pairs {
        oracle.insert_preimage(
            PreimageKey::new_keccak256(slot_hash_pair_hash(*slot).0),
            bincode::serialize(&(*hash, *parent_hash))?,
        );
    }

    // save l2 blocks
    for (slot, block) in &storage_items.l2_blocks {
        let mut buf = Vec::new();
        block.encode(&mut buf);
        oracle.insert_preimage(
            PreimageKey::new_keccak256(*keccak256(slot.to_be_bytes().as_ref())),
            buf,
        );
    }

    Ok(())
}

pub(crate) fn update_execution_storage_items(
    executor: &SharedExecutor,
    storage_items: &mut ExecutionStorageItems,
) -> Result<()> {
    let slot = executor.latest_slot()?;
    let l2_block_info = executor.l2_block_info_by_number_immut(slot)?;
    executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        storage_items.init_accounts.insert(slot, soon_accounts);
        storage_items
            .sysvar_accounts
            .insert(slot, s.export_sysvars()?);

        storage_items.slot_hash_pairs.insert(
            slot,
            (
                l2_block_info.block_info.hash,
                l2_block_info.block_info.parent_hash,
            ),
        );
        Ok(())
    })?;
    Ok(())
}

pub(crate) async fn blocks_to_execution_cache(
    producer: &mut Producer<SharedExecutor, MockInstant>,
    identity: &Keypair,
    metadata: &TokenMetadata,
    complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
) -> Result<(BootInfo, Vec<Arc<Execution>>, ExecutionStorageItems)> {
    let mut executions = Vec::new();
    let mut boot_info = BootInfo {
        l1_head: B256::ZERO,
        agreed_l2_output_root: B256::ZERO,
        claimed_l2_output_root: B256::ZERO,
        agreed_l2_block_number: 1,
        claimed_l2_block_number: 3,
        chain_id: 0,
        rollup_config: SoonRollupConfig::default(),
    };
    let mut storage_items = ExecutionStorageItems::default();
    let mut executor = producer.get_executor().clone();
    executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        storage_items
            .init_accounts
            .insert(s.current_slot(), soon_accounts);
        Ok(())
    })?;
    let agreed_output = current_executor_state_root(&executor)?;
    let init_slot = executor.latest_slot()?;
    let slot_1_head = executor.l2_block_info_by_number_immut(init_slot)?;
    storage_items.safe_head = slot_1_head;
    storage_items
        .l2_blocks
        .insert(init_slot, executor.block_by_number(init_slot).await?);
    info!(
        "storage safe head: {:?}, l2 block: {:?}",
        storage_items.safe_head,
        storage_items.l2_blocks.get(&init_slot).unwrap()
    );
    boot_info.agreed_l2_output_root = agreed_output;

    // === slot 2
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
    update_execution_storage_items(&executor, &mut storage_items)?;

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
    update_execution_storage_items(&executor, &mut storage_items)?;

    let slot_3_head = executor.l2_block_info_by_number_immut(executor.latest_slot()?)?;
    let execution = to_execution(raw, agreed_output, claimed_output, slot_3_head)?;
    executions.push(Arc::new(execution));

    Ok((boot_info, executions, storage_items))
}
