use super::{
    fetch_info_and_update_execution_storage_items, new_soon, ExecutionStorageItems, TokenMetadata,
};
use crate::client::soon_test::{to_execution, L1_NUMBER};
use alloy_primitives::{keccak256, B256};
use alloy_rlp::{BytesMut, Encodable};
use anyhow::{ensure, Result};
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use kona_executor::{cal_init_state_root_hash, cal_soon_accounts_hash, cal_svm_clock_timestamp};
use kona_host::MemoryKeyValueStore;
use kona_host::OnlineHostBackend;
use kona_host::PreimageServer;
use kona_preimage::BidirectionalChannel;
use kona_preimage::HintReader;
use kona_preimage::OracleServer;
use kona_preimage::PreimageKey;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signature::Signer;
use solana_sdk::system_transaction;
use soon_mpt_handler::MptHandler;
use soon_node::node::tests::{new_derive_block_with_mock_l1, MockEthL1Node};
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::mpt::MptRunner,
    node::{producer::Producer, tests::create_spl_tx},
};
use soon_primitives::mpt::MptUpdatingItem;
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo, RawBlock},
    rollup_config::SoonRollupConfig,
};
use spl_token::state::Mint;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub type E2eSoonProducer = Producer<SharedExecutor, MockInstant>;
pub type SharedMockL1 = Arc<RwLock<MockEthL1Node>>;

/// An all-in-one environment holding every component for e2e fraud proof testing between
/// Soon Execution and Kona Execution.
pub struct E2eKailuaSoonEnvironment {
    pub e2e_producer: E2eSoonProducer,
    pub mpt_runner: MptRunner,
    pub mpt_signal: Sender<MptUpdatingItem>,
    pub identity: Arc<Keypair>,
    pub metadata: TokenMetadata,
    pub complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
    pub l1_node: SharedMockL1,
    pub mints: Vec<Keypair>,
    pub chain_provider: hints::E2EChainProvider,
}

pub async fn init_soon_env(relative_to_soon: Option<&str>) -> Result<E2eKailuaSoonEnvironment> {
    // init soon producer.
    let mut mock_l1_node = MockEthL1Node::new(L1_NUMBER, 12);
    let temp = tempfile::tempdir()?;
    let (mut e2e_producer, identity, metadata, complete_receiver, mints) =
        new_soon(temp.path(), relative_to_soon, &mut mock_l1_node)?;
    let l1_node = Arc::new(RwLock::new(mock_l1_node));

    // init mpt calculation.
    let mpt_path = tempfile::tempdir()?;
    let exit = e2e_producer
        .get_executor()
        .storage_query(|storage| Ok(storage.exit.clone()))?;

    let (sender, r, s, _) = e2e_producer
        .get_executor()
        .storage_query(|storage| Ok(storage.signal_hub.mpt_update_chanel.clone().unwrap()))?;
    let mpt_runner = MptRunner::new(mpt_path.path(), r, s, exit)?;

    // init hint system.
    let mut kv = Arc::new(RwLock::new(MemoryKeyValueStore::new()));
    let hint = BidirectionalChannel::new()?;
    let preimage = BidirectionalChannel::new()?;
    let chain_provider = hints::E2EChainProvider {
        l1: l1_node.clone(),
        executor: e2e_producer.get_executor().clone(),
        mpt: mpt_runner.inner_handler(),
    };
    let backend = OnlineHostBackend::new(
        hints::E2EOnlineHostBackendCfg {},
        kv,
        chain_provider.clone(),
        hints::E2EHintHandler {},
    );
    tokio::task::spawn(
        PreimageServer::new(
            OracleServer::new(preimage.host),
            HintReader::new(hint.host),
            Arc::new(backend),
        )
        .start(),
    );

    Ok(E2eKailuaSoonEnvironment {
        e2e_producer,
        mpt_runner,
        mpt_signal: sender,
        identity,
        metadata,
        complete_receiver,
        l1_node,
        mints,
        chain_provider,
    })
}

/// promote_multi_tx will promote more than 200 random blocks
/// including modify `data` and `lamports` of all accounts.
pub async fn promote_multi_tx(env: &mut E2eKailuaSoonEnvironment) -> Result<()> {
    let blocks = 50;
    let last_blockhash = env
        .e2e_producer
        .get_executor()
        .storage_query(|s| Ok(s.current_bank().last_blockhash()))?;

    for i in 0..blocks {
        let from = env.mints[i % env.mints.len()].insecure_clone();
        let to = env.mints[(i + 1) % env.mints.len()].pubkey();
        let tx = system_transaction::transfer(&from, &to, LAMPORTS_PER_SOL, last_blockhash);
        env.e2e_producer.add_tx(tx)?;
        env.e2e_producer.mine_with_block(None)?;
        env.complete_receiver.try_recv()?;

        // finalize at once.
        env.mpt_signal
            .send(MptUpdatingItem::UpdateFinalizedSlot((i + 1) as u64))?;
    }
    Ok(())
}

pub mod hints {
    use super::*;
    use anyhow::{bail, Ok};
    use async_trait::async_trait;
    use solana_sdk::account::{AccountSharedData, WritableAccount};
    use soon_mpt_primitives::constants::KECCAK_EMPTY;
    use soon_primitives::{
        l2blocks::L2Block,
        mpt::{AccountWithTrie, WrappedSolanaAccount},
        output_root::OutputRoot,
    };
    use std::sync::Arc;

    use kona_host::{HintHandler, OnlineHostBackendCfg, SharedKeyValueStore};
    use kona_proof::{Hint, HintType};
    use soon_node::{derive::driver::L2ChainProviderImmutable, executor::SharedExecutor};

    use crate::client::soon_test::e2e::E2eKailuaSoonEnvironment;

    pub struct E2EOnlineHostBackendCfg {}

    impl OnlineHostBackendCfg for E2EOnlineHostBackendCfg {
        type HintType = HintType;

        type Providers = E2EChainProvider;
    }

    #[derive(Clone)]
    pub struct E2EChainProvider {
        pub l1: SharedMockL1,
        pub executor: SharedExecutor,
        pub mpt: Arc<MptHandler>,
    }

    impl E2EChainProvider {
        fn output_at_block(&self, slot: u64) -> Result<OutputRoot> {
            let slot = self.mpt.get_aligned_slot(slot)?;
            let block_hash = B256::from_slice(
                bs58::decode(self.executor.block_by_number_immut(slot)?.block.blockhash)
                    .into_vec()?
                    .as_slice(),
            );
            let state_root = self.mpt.query_state_root(slot)?;
            let withdrawal_root = self.mpt.query_withdrawal_root(slot)?;

            Ok(OutputRoot {
                state_root,
                bridge_storage_root: withdrawal_root,
                block_hash: block_hash,
            })
        }

        fn get_tried_account_proof(&self, address: B256, slot: u64) -> Result<AccountWithTrie> {
            let slot = self.mpt.get_aligned_slot(slot)?;
            let state_proof = self.mpt.proof_of_state_root(address, slot)?;
            if !state_proof.encoded_account.is_some() {
                return Ok(AccountWithTrie {
                    block_number: 0,
                    proofs: vec![],
                    withdrawal_proofs: vec![],
                    account: None,
                });
            }

            let account = state_proof.raw_account.unwrap();
            let mut raw_account =
                AccountSharedData::new(0, 0, &Pubkey::from(account.owner.0.to_bytes()));
            raw_account.set_lamports(account.lamports);
            raw_account.set_executable(account.executable);
            raw_account.set_rent_epoch(account.rent_epoch);
            if account.data != KECCAK_EMPTY {
                let raw = self
                    .mpt
                    .query_historical_raw_account(address, slot)?
                    .unwrap();
                raw_account.set_data_from_slice(&raw.data.as_slice());
            }

            let bridge = Pubkey::try_from(
                bs58::decode("Bridge1111111111111111111111111111111111111").into_vec()?,
            )
            .unwrap();
            let mut withdrawal_proofs = vec![];
            if account.owner.0.to_string() == bridge.to_string() {
                let p = self.mpt.proof_of_withdrawal_root(address, slot)?;
                withdrawal_proofs = p.proof.into_iter().map(|b| b.to_vec()).collect::<Vec<_>>();
            }

            Ok(AccountWithTrie {
                block_number: slot,
                proofs: state_proof
                    .proof
                    .into_iter()
                    .map(|b| b.to_vec())
                    .collect::<Vec<_>>(),
                withdrawal_proofs,
                account: Some(WrappedSolanaAccount(raw_account)),
            })
        }

        fn get_block_by_number(&self, slot: u64) -> Result<L2Block> {
            Ok(L2Block {
                previous_blockhash: todo!(),
                blockhash: todo!(),
                parent_slot: todo!(),
                block_time: todo!(),
                block_height: todo!(),
                transactions: todo!(),
            })
        }

        fn get_bank_hash(&self, slot: u64) -> Result<Option<String>> {
            Ok(None)
        }

        fn get_block_time(&self, slot: u64) -> Result<Option<i64>> {
            Ok(None)
        }
    }

    pub struct E2EHintHandler {}

    #[async_trait]
    impl HintHandler for E2EHintHandler {
        type Cfg = E2EOnlineHostBackendCfg;

        /// Fetches data in response to a hint.
        async fn fetch_hint(
            hint: Hint<<Self::Cfg as OnlineHostBackendCfg>::HintType>,
            cfg: &Self::Cfg,
            providers: &<Self::Cfg as OnlineHostBackendCfg>::Providers,
            kv: SharedKeyValueStore,
        ) -> Result<()> {
            match hint.ty {
                HintType::L1BlockHeader => {}
                HintType::L1Transactions => {}
                HintType::L1Receipts => {}
                HintType::L1Blob => {}
                HintType::DAProxyBlob => {}
                HintType::L1Precompile => {}
                HintType::StartingL2Output => {
                    ensure!(
                        hint.data.len() == 8,
                        "Invalid hint data length for starting l2 output"
                    );
                    let block_number = u64::from_be_bytes(hint.data.as_ref().try_into()?);

                    let output_res: OutputRoot = providers.output_at_block(block_number)?;
                    info!("output_res:{}", output_res);
                    let output_root_hash = output_res.hash();
                    let mut kv_write_lock = kv.write().await;
                    kv_write_lock.set(
                        PreimageKey::new_keccak256(*output_root_hash).into(),
                        output_res.encode().into(),
                    )?;
                }
                HintType::L2StateNode => {}
                HintType::L2AccountProof => {
                    // block number + hashed address<b256>
                    ensure!(
                        hint.data.len() == 8 + 32,
                        "Invalid hint req for L2AccountProof"
                    );
                    let block_number = u64::from_be_bytes(hint.data.as_ref()[..8].try_into()?);
                    let hashed_address = B256::from_slice(&hint.data.as_ref()[8..40]);

                    let tried_account =
                        providers.get_tried_account_proof(hashed_address, block_number)?;
                    // need to write account + trie proof node into kv.
                    let mut out_buf = BytesMut::default();
                    if let Some(account) = tried_account.account {
                        Encodable::encode(&account, &mut out_buf);
                    }
                    let mut kv_lock = kv.write().await;
                    kv_lock.set(
                        PreimageKey::new_l2_account_proof(hashed_address.into()).into(),
                        out_buf.into(),
                    )?;
                    tried_account.proofs.into_iter().try_for_each(|node| {
                        let node_hash = keccak256::<&[u8]>(node.as_ref());
                        let key = PreimageKey::new_keccak256(*node_hash);
                        kv_lock.set(key.into(), node.into())?;
                        Ok(())
                    })?;
                    tried_account
                        .withdrawal_proofs
                        .into_iter()
                        .try_for_each(|node| {
                            let node_hash = keccak256::<&[u8]>(node.as_ref());
                            let key = PreimageKey::new_keccak256(*node_hash);
                            kv_lock.set(key.into(), node.into())?;
                            Ok(())
                        })?;
                }
                HintType::L2BlockData => {
                    ensure!(
                        hint.data.len() == 8,
                        "Invalid hint data length for l2 block data"
                    );

                    let block_number = u64::from_be_bytes(hint.data.as_ref()[..8].try_into()?);

                    let block = providers.get_block_by_number(block_number)?;
                    let mut out_buf = BytesMut::default();
                    Encodable::encode(&block, &mut out_buf);
                    let mut kv_lock = kv.write().await;
                    kv_lock.set(
                        PreimageKey::new_block_slot(block_number).into(),
                        out_buf.into(),
                    )?;
                }
                HintType::L2BankHash => {
                    ensure!(
                        hint.data.len() == 8,
                        "Invalid hint data length for l2 bank hash"
                    );
                    info!("handle L2BankHash request.");
                    let block_number = u64::from_be_bytes(hint.data.as_ref()[..8].try_into()?);
                    let bank_hash = providers.get_bank_hash(block_number)?;
                    info!("bank_hash:{:?}", bank_hash);
                    let bank_hash_bytes = match bank_hash {
                        Some(hash) => bs58::decode(hash.as_str()).into_vec().unwrap(),
                        None => vec![],
                    };
                    info!("bank_hash_bytes len:{}", bank_hash_bytes.len());
                    let mut kv_lock = kv.write().await;
                    kv_lock.set(
                        PreimageKey::new_l2_bank_hash(block_number).into(),
                        bank_hash_bytes,
                    )?;
                }
                HintType::L2BlockTime => {
                    ensure!(
                        hint.data.len() == 8,
                        "Invalid hint data length for l2 block time"
                    );
                    let block_number = u64::from_be_bytes(hint.data.as_ref()[..8].try_into()?);
                    let block_time = providers.get_block_time(block_number)?;
                    let block_time_bytes = match block_time {
                        Some(time) => time.to_be_bytes().to_vec(),
                        None => vec![],
                    };
                    let mut kv_lock = kv.write().await;
                    kv_lock.set(
                        PreimageKey::new_l2_block_time(block_number).into(),
                        block_time_bytes,
                    )?;
                }
            }

            Ok(())
        }
    }
}

pub mod trie_db {
    use kona_executor::{TrieDB, TrieDBProvider};
    use kona_mpt::{TrieHinter, TrieProvider};

    use crate::client::soon_test::e2e::hints;

    #[derive(thiserror::Error, Debug, Eq, PartialEq)]
    #[error("TestE2EError: {0}")]
    pub struct TestE2EError(&'static str);

    impl TrieProvider for hints::E2EChainProvider {
        type Error = TestE2EError;

        fn trie_node_by_hash(
            &self,
            key: alloy_primitives::B256,
        ) -> Result<kona_mpt::TrieNode, Self::Error> {
            todo!()
        }

        fn bank_hash(&self, block_number: u64) -> Result<alloy_primitives::B256, Self::Error> {
            todo!()
        }

        fn block_time(&self, block_number: u64) -> Result<i64, Self::Error> {
            todo!()
        }
    }

    impl TrieDBProvider for hints::E2EChainProvider {
        fn data_by_hash(
            &self,
            code_hash: alloy_primitives::B256,
        ) -> Result<alloy_primitives::Bytes, Self::Error> {
            unimplemented!("data by hash")
        }
    }

    impl TrieHinter for hints::E2EChainProvider {
        type Error = TestE2EError;

        fn hint_trie_node(&self, hash: alloy_primitives::B256) -> Result<(), Self::Error> {
            todo!()
        }

        fn hint_account_proof(
            &self,
            pubkey: &solana_sdk::pubkey::Pubkey,
            block_number: u64,
        ) -> Result<(), Self::Error> {
            todo!()
        }

        fn hint_bank_hash(&self, block_number: u64) -> Result<(), Self::Error> {
            todo!()
        }

        fn hint_block_time(&self, block_number: u64) -> Result<(), Self::Error> {
            todo!()
        }
    }

    pub type E2ETrieDB = TrieDB<hints::E2EChainProvider, hints::E2EChainProvider>;
}
