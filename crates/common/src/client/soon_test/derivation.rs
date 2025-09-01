use crate::client::soon_test::{
    fetch_info_and_update_execution_storage_items, ExecutionStorageItems, L1_NUMBER,
};
use crate::{oracle::WitnessOracle, test::mock::MockOracle};
use alloy_primitives::bytes::BytesMut;
use alloy_primitives::{address, keccak256, Address, B256};
use alloy_rlp::Encodable;
use anyhow::Result;
use batcher::driver::types::FrameID;
use batcher::{ChannelOut, FrameData, SingularChannelOut, TransactionData};
use bridge::pda::{spl_token_mint_pubkey, spl_token_owner_pubkey};
use crossbeam_channel::Receiver;
use kona_preimage::PreimageKey;
use kona_proof::BootInfo;
use solana_sdk::{
    account::ReadableAccount, program_pack::Pack, signature::Keypair, signer::Signer,
};
use soon_derive::traits::L2ChainProvider;
use soon_node::node::tests::{new_derive_block_with_mock_l1, MockEthL1Node};
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::{producer::Producer, tests::create_spl_tx},
};
use soon_primitives::blocks::L1Transaction;
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    rollup_config::SoonRollupConfig,
};
use spl_token::state::Mint;
use tracing::info;

use super::execution::executions_save_to_oracle;
use super::{new_soon, DerivationStorageItems, TokenMetadata};

#[allow(dead_code)]
pub async fn soon_to_derivation(relative_to_soon: Option<&str>) -> Result<(BootInfo, MockOracle)> {
    let mut mock_l1_node = MockEthL1Node::new(L1_NUMBER, 12);
    let temp = tempfile::tempdir()?;
    let (mut producer, identity, metadata, complete_receiver, _) =
        new_soon(temp.path(), relative_to_soon, &mut mock_l1_node)?;

    let (boot_info, oracle_storage_items) = blocks_to_derivation_cache(
        &mut producer,
        &identity,
        &metadata,
        complete_receiver,
        &mut mock_l1_node,
    )
    .await?;
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
        let mut buf = BytesMut::default();
        Encodable::encode(header, &mut buf);

        oracle.insert_preimage(PreimageKey::new_keccak256(hash.0), buf.clone().into());
        oracle.insert_preimage(
            PreimageKey::new_keccak256(*keccak256(header.number.to_be_bytes().as_ref())),
            buf.into(),
        );
    }

    // save l1 transaction
    for (hash, l1_txs) in &storage_items.l1_transactions {
        let mut key_data = "l1_transaction".to_string().into_bytes();
        let mut hash_data = hash.0.to_vec();
        key_data.append(&mut hash_data);

        let mut buf = BytesMut::default();
        Encodable::encode(l1_txs, &mut buf);

        oracle.insert_preimage(
            PreimageKey::new_keccak256(keccak256(key_data.as_slice()).0),
            buf.clone().into(),
        );
    }

    // save da data
    for (data_hash, data) in &storage_items.da_data {
        oracle.insert_preimage(PreimageKey::new_keccak256(data_hash.0), data.clone());
    }
    Ok(())
}

pub(crate) async fn blocks_to_derivation_cache(
    producer: &mut Producer<SharedExecutor, MockInstant>,
    identity: &Keypair,
    metadata: &TokenMetadata,
    complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
    l1_node: &mut MockEthL1Node,
) -> Result<(BootInfo, DerivationStorageItems)> {
    let batch_inbox_address = address!("0xfF000000000000000000000000000000000000FF");
    let rollup_config = SoonRollupConfig {
        seq_window_size: 1000, //prevent generating empty batch
        batch_inbox_address,
        channel_size: 100_000_000, //prevent batch size exceeding channel_size
        max_sequencer_drift: 10_000_000_000, //prevent batch exceeding sequencer time drift
        ..Default::default()
    };
    let mut boot_info = BootInfo {
        l1_head: B256::ZERO,
        agreed_l2_output_root: B256::ZERO,
        claimed_l2_output_root: B256::ZERO,
        agreed_l2_block_number: 1,
        claimed_l2_block_number: 3,
        chain_id: 0,
        rollup_config,
    };
    let mut storage_items = DerivationStorageItems::default();
    let mut executor = producer.get_executor().clone();

    // update storage items for slot 1
    let (head, state_root_1, _) =
        fetch_info_and_update_execution_storage_items(&mut executor, &mut storage_items.execution)
            .await?;
    info!("soon slot 1 state root: {:?}", state_root_1);
    storage_items.execution.safe_head = head;
    info!(
        "execution safe head: {:?}",
        storage_items.execution.safe_head
    );
    boot_info.agreed_l2_output_root = state_root_1;
    // set l1 origin head
    boot_info.l1_head = head.l1_origin.hash;
    // save l1 heads
    let header = l1_node
        .get_block_header(head.l1_origin.number)
        .unwrap()
        .clone();
    storage_items.l1_heads.insert(head.l1_origin.hash, header);

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
    let (head, state_root_2, _) =
        fetch_info_and_update_execution_storage_items(&mut executor, &mut storage_items.execution)
            .await?;
    info!("soon slot 2 state root: {:?}", state_root_2);
    // save l1 heads
    let header = l1_node
        .get_block_header(head.l1_origin.number)
        .unwrap()
        .clone();
    storage_items.l1_heads.insert(head.l1_origin.hash, header);

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
    let (head, state_root_3, _) =
        fetch_info_and_update_execution_storage_items(&mut executor, &mut storage_items.execution)
            .await?;
    info!("soon slot 3 state root: {:?}", state_root_3);
    boot_info.claimed_l2_output_root = state_root_3;
    boot_info.claimed_l2_block_number = producer.get_executor().latest_slot()?;
    info!("boot info: {:?}", boot_info);
    // save l1 heads
    let header = l1_node
        .get_block_header(head.l1_origin.number)
        .unwrap()
        .clone();
    storage_items.l1_heads.insert(head.l1_origin.hash, header);

    // save slot 2-3 to batch data
    let mut channel_out = SingularChannelOut::new(1_000_000_000, 9);
    for i in 2..=3 {
        let block = executor.block_by_number(i).await.unwrap();
        ChannelOut::add_block(&mut channel_out, block).unwrap();
    }
    channel_out.close().unwrap();
    let mut buf: Vec<u8> = Vec::new();
    let (frame_number, is_last) =
        ChannelOut::output_frame(&mut channel_out, &mut buf, 1_000_000_000).unwrap();
    assert!(is_last);
    assert_eq!(frame_number, 0);
    let frame = FrameData {
        id: FrameID {
            ch_id: ChannelOut::id(&channel_out),
            frame_num: frame_number,
        },
        data: buf,
    };
    let mut tx_data = TransactionData::default();
    tx_data.frames.push(frame);
    let data = tx_data.call_data();

    // save batch data to da server provider. hash -> batch data
    let da_data_hash = keccak256(data.as_slice());
    storage_items.da_data.insert(da_data_hash, data);

    // save hash in l1 transaction
    let tx: L1Transaction = L1Transaction {
        hash: B256::ZERO,
        from: Address::ZERO,
        to: Some(batch_inbox_address),
        input: da_data_hash.as_slice().to_vec(),
    };

    // save l1 transaction for l1 block
    let new_l1_block = l1_node.mine_block_with_transactions();
    assert_eq!(new_l1_block.number, 102);
    let header = l1_node
        .get_block_header(new_l1_block.number)
        .unwrap()
        .clone();
    storage_items.l1_heads.insert(new_l1_block.hash, header);
    storage_items
        .l1_transactions
        .insert(new_l1_block.hash, vec![tx]);

    Ok((boot_info, storage_items))
}
