use crate::client::soon_test::{current_executor_state_root, L1_NUMBER};
use crate::{oracle::WitnessOracle, test::mock::MockOracle};
use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rlp::Encodable;
use anyhow::Result;
use bridge::pda::{spl_token_mint_pubkey, spl_token_owner_pubkey};
use crossbeam_channel::Receiver;
use fraud_executor::accounts::SoonAccounts;
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
        tests::{create_derived_deposit_erc20_tx, create_spl_tx, new_derive_block, DEPOSIT_AMOUNT},
    },
};
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    rollup_config::SoonRollupConfig,
};
use spl_token::state::Mint;
use tracing::info;

use super::execution::{executions_save_to_oracle, update_execution_storage_items};
use super::{new_soon, DerivationStorageItems, TokenMetadata};

#[allow(dead_code)]
pub async fn soon_to_derivation() -> Result<(BootInfo, MockOracle)> {
    let temp = tempfile::tempdir()?;
    let (mut producer, identity, metadata, complete_receiver) = new_soon(temp.path())?;

    let (boot_info, oracle_storage_items) =
        blocks_to_derivation_cache(&mut producer, &identity, &metadata, complete_receiver).await?;
    let mut oracle = MockOracle::new(boot_info.clone());
    derivations_save_to_oracle(&mut oracle, &boot_info, &oracle_storage_items)?;
    Ok((boot_info, oracle))
}

fn derivations_save_to_oracle(
    oracle: &mut MockOracle,
    boot_info: &BootInfo,
    storage_items: &DerivationStorageItems,
) -> Result<()> {
    executions_save_to_oracle(oracle, boot_info, &storage_items.execution)?;

    // save l1 heads
    for (hash, header) in &storage_items.l1_heads {
        let mut buf = Vec::new();
        header.encode(&mut buf);
        oracle.insert_preimage(PreimageKey::new_keccak256(hash.0), buf);
    }

    Ok(())
}

fn update_derivation_storage_items(
    executor: &SharedExecutor,
    storage_items: &mut DerivationStorageItems,
) -> Result<()> {
    update_execution_storage_items(executor, &mut storage_items.execution)?;
    Ok(())
}

pub(crate) async fn blocks_to_derivation_cache(
    producer: &mut Producer<SharedExecutor, MockInstant>,
    identity: &Keypair,
    metadata: &TokenMetadata,
    complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
) -> Result<(BootInfo, DerivationStorageItems)> {
    let mut boot_info = BootInfo {
        l1_head: B256::ZERO,
        agreed_l2_output_root: B256::ZERO,
        claimed_l2_output_root: B256::ZERO,
        agreed_l2_block_number: 1,
        claimed_l2_block_number: 3,
        chain_id: 0,
        rollup_config: SoonRollupConfig::default(),
    };
    let mut storage_items = DerivationStorageItems::default();
    let mut executor = producer.get_executor().clone();
    executor.storage_query(|s| {
        let soon_accounts = SoonAccounts::try_from(s)?;
        storage_items
            .execution
            .init_accounts
            .insert(s.current_slot(), soon_accounts);
        Ok(())
    })?;
    let agreed_output = current_executor_state_root(&executor)?;
    let init_slot = executor.latest_slot()?;
    let slot_1_head = executor.l2_block_info_by_number_immut(init_slot)?;
    storage_items.execution.safe_head = slot_1_head;
    storage_items
        .execution
        .l2_blocks
        .insert(init_slot, executor.block_by_number(init_slot).await?);

    info!(
        "execution safe head: {:?}",
        storage_items.execution.safe_head
    );
    boot_info.agreed_l2_output_root = agreed_output;
    // set l1 origin head
    boot_info.l1_head = slot_1_head.l1_origin.hash;
    storage_items.l1_heads.insert(
        slot_1_head.l1_origin.hash,
        Header {
            number: slot_1_head.l1_origin.number,
            ..Default::default()
        },
    );

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
    update_derivation_storage_items(&executor, &mut storage_items)?;

    // === slot 3
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
    update_derivation_storage_items(&executor, &mut storage_items)?;

    Ok((boot_info, storage_items))
}
