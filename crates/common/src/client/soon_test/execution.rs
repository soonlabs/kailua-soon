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
    cal_init_state_root_hash, cal_soon_accounts_hash, cal_svm_bank_hash, cal_svm_clock_timestamp,
};
use kona_preimage::PreimageKey;
use kona_proof::BootInfo;
use solana_sdk::{
    account::ReadableAccount, program_pack::Pack, signature::Keypair, signer::Signer,
};
use soon_node::node::tests::{new_derive_block_with_mock_l1, MockEthL1Node};
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::{producer::Producer, tests::create_spl_tx},
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
    let mut mock_l1_node = MockEthL1Node::new(L1_NUMBER);
    let temp = tempfile::tempdir()?;
    let (mut producer, identity, metadata, complete_receiver, _) =
        new_soon(temp.path(), relative_to_soon, &mut mock_l1_node)?;

    let (boot_info, executions, oracle_storage_items) = blocks_to_execution_cache(
        &mut producer,
        &identity,
        &metadata,
        complete_receiver,
        &mut mock_l1_node,
    )
    .await?;
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

    // save soon accounts
    storage_items
        .soon_accounts
        .iter()
        .for_each(|(slot, accounts)| {
            oracle.insert_preimage(
                PreimageKey::new_keccak256(cal_soon_accounts_hash(*slot).0),
                bincode::serialize(accounts).unwrap(),
            );
            oracle.insert_preimage(
                PreimageKey::new_keccak256(cal_init_state_root_hash(*slot).0),
                accounts.state_root().to_vec(),
            )
        });

    // save l2 blocks
    for (slot, block) in &storage_items.l2_blocks {
        let mut buf = BytesMut::default();
        Encodable::encode(block, &mut buf);
        oracle.insert_preimage(
            PreimageKey::new_keccak256(*keccak256(slot.to_be_bytes().as_ref())),
            buf.into(),
        );
    }

    // save bank hashes
    for (slot, hash) in &storage_items.bank_hashes {
        oracle.insert_preimage(
            PreimageKey::new_keccak256(cal_svm_bank_hash(*slot).0),
            bincode::serialize(hash)?,
        );
    }

    // save clock timestamps
    for (slot, timestamp) in &storage_items.clock_timestamps {
        oracle.insert_preimage(
            PreimageKey::new_keccak256(cal_svm_clock_timestamp(*slot).0),
            bincode::serialize(timestamp)?,
        );
    }

    Ok(())
}

pub(crate) async fn blocks_to_execution_cache(
    producer: &mut Producer<SharedExecutor, MockInstant>,
    identity: &Keypair,
    metadata: &TokenMetadata,
    complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
    l1_node: &mut MockEthL1Node,
) -> Result<(BootInfo, Vec<Arc<Execution>>, ExecutionStorageItems)> {
    let mut executions = Vec::new();
    let mut boot_info = BootInfo {
        l1_head: B256::ZERO,
        agreed_l2_output_root: B256::ZERO,
        claimed_l2_output_root: B256::ZERO,
        agreed_l2_block_number: 1,
        claimed_l2_block_number: 3,
        chain_id: 0,
        rollup_config: SoonRollupConfig {
            sequencer_schedules: vec![(0, identity.pubkey())],
            ..Default::default()
        },
    };
    let mut storage_items = ExecutionStorageItems::default();
    let mut executor = producer.get_executor().clone();

    // update storage items for slot 1
    let (head, state_root_1, _) =
        fetch_info_and_update_execution_storage_items(&mut executor, &mut storage_items).await?;
    info!("soon slot 1 state root: {:?}", state_root_1);
    storage_items.safe_head = head;
    info!("storage safe head: {:?}", storage_items.safe_head);
    boot_info.agreed_l2_output_root = state_root_1;

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
    // let to_account_data = executor.get_account_by_slot(2, &metadata.to.pubkey())?;
    // assert_eq!(to_account_data.lamports(), DEPOSIT_AMOUNT);
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

    // update storage items for slot 2
    let (head, state_root_2, l2_block) =
        fetch_info_and_update_execution_storage_items(&mut executor, &mut storage_items).await?;
    info!("soon slot 2 state root: {:?}", state_root_2);
    // get execution
    let execution = to_execution(l2_block, state_root_1, state_root_2, head)?;
    executions.push(Arc::new(execution));

    // === slot 3
    let derive_block_2 = new_derive_block_with_mock_l1(l1_node, metadata.to.pubkey());
    // let mut derive_block_2 = new_derive_block(metadata.to.pubkey(), L1_NUMBER + 1);
    // deposit erc20 token
    // derive_block_2
    //     .deposit_txs
    //     .push(create_derived_deposit_erc20_tx(
    //         metadata.remote_token,
    //         metadata.to.pubkey(),
    //     ));
    let raw = RawBlock::try_init(derive_block_2, 0, &Default::default())?;
    producer.mine_with_block(Some(raw.clone()))?;
    complete_receiver.try_recv()?;

    // update storage items for slot 3
    let (head, state_root_3, l2_block) =
        fetch_info_and_update_execution_storage_items(&mut executor, &mut storage_items).await?;
    info!("soon slot 3 state root: {:?}", state_root_3);
    // get execution
    let execution = to_execution(l2_block, state_root_2, state_root_3, head)?;
    executions.push(Arc::new(execution));

    // update boot info
    boot_info.claimed_l2_output_root = state_root_3;
    boot_info.claimed_l2_block_number = producer.get_executor().latest_slot()?;
    info!("boot info: {:?}", boot_info);

    Ok((boot_info, executions, storage_items))
}
