use super::{new_soon, TokenMetadata};
use crate::client;
use crate::client::soon_test::e2e::hints::*;
use crate::client::soon_test::{to_execution, L1_NUMBER};
use crate::executor::Execution;
use alloy_primitives::{keccak256, B256};
use alloy_rlp::{BytesMut, Encodable};
use anyhow::{bail, ensure, Context, Result};
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use fraud_executor::accounts::SoonAccounts;
use kona_driver::Executor;
use kona_executor::{L2BlockBuilder, TrieDB};
use kona_host::MemoryKeyValueStore;
use kona_host::OnlineHostBackend;
use kona_host::PreimageServer;
use kona_preimage::errors::PreimageOracleResult;
use kona_preimage::PreimageKey;
use kona_preimage::{BidirectionalChannel, CommsClient};
use kona_preimage::{HintReader, PreimageOracleClient};
use kona_preimage::{HintWriterClient, OracleServer};
use kona_proof::executor::KonaExecutor;
use kona_proof::BootInfo;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signature::Signer;
use solana_sdk::system_transaction;
use soon_derive::traits::{ChainProvider, L2ChainProvider};
use soon_mpt_handler::MptHandler;
use soon_node::node::tests::MockEthL1Node;
use soon_node::{
    derive::mock::MockInstant,
    executor::{ExecutorOperator, SharedExecutor},
    node::mpt::MptRunner,
    node::producer::Producer,
};
use soon_primitives::blocks::L2BlockHeader;
use soon_primitives::l2blocks::L2Block;
use soon_primitives::mpt::MptUpdatingItem;
use soon_primitives::{
    blocks::{BlockInfo, L2BlockInfo},
    rollup_config::SoonRollupConfig,
};
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::RwLock;
use tracing::info;

pub type E2eSoonProducer = Producer<SharedExecutor, MockInstant>;
pub type SharedMockL1 = Arc<RwLock<MockEthL1Node>>;

/// An all-in-one environment holding every component for e2e fraud proof testing between
/// Soon Execution and Kona Execution.
pub struct E2EKailuaSoonEnvironment {
    pub e2e_producer: E2eSoonProducer,
    pub mpt_runner: MptRunner,
    pub mpt_signal: Sender<MptUpdatingItem>,
    pub identity: Arc<Keypair>,
    pub metadata: TokenMetadata,
    pub complete_receiver: Receiver<(L2BlockInfo, Option<BlockInfo>)>,
    pub l1_node: SharedMockL1,
    pub mints: Vec<Keypair>,
    pub chain_provider: E2EChainProvider,
    pub oracel: E2EOracle,
    pub soon_path: TempDir,
}

pub async fn init_soon_env(relative_to_soon: Option<&str>) -> Result<E2EKailuaSoonEnvironment> {
    // init soon producer.
    let mut mock_l1_node = MockEthL1Node::new(L1_NUMBER, 12);
    let temp = tempfile::tempdir()?;
    let (mut e2e_producer, identity, metadata, complete_receiver, mints) =
        new_soon(temp.path(), relative_to_soon, &mut mock_l1_node)?;

    let l1_node = Arc::new(RwLock::new(mock_l1_node));

    // init mpt calculation.
    let mpt_path = tempfile::tempdir()?;
    let executor = e2e_producer.get_executor();
    let exit = executor.storage_query(|storage| Ok(storage.exit.clone()))?;

    let (sender, r, s, _) = executor
        .storage_query(|storage| Ok(storage.signal_hub.mpt_update_chanel.clone().unwrap()))?;
    let mpt_runner = MptRunner::new(mpt_path.path(), r, s, exit)?;
    mpt_runner.clone().run();
    // finalize 0 and 1 slot.
    sender.send(MptUpdatingItem::UpdateFinalizedSlot(0u64))?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let state_root = mpt_runner.inner_handler().query_state_root(0)?;
    let withdrawal_root = mpt_runner.inner_handler().query_withdrawal_root(0)?;
    info!("init state_root {state_root}, withdrawal root {withdrawal_root}");

    // init hint backend.
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
            OracleServer::new(preimage.client),
            HintReader::new(hint.host),
            Arc::new(backend),
        )
        .start(),
    );

    // init trie-db frontend
    let oracel = E2EOracle {
        hint_writer: HintWriter::new(hint.client),
        preimage_reader: OracleReader::new(preimage.host),
    };

    Ok(E2EKailuaSoonEnvironment {
        e2e_producer,
        mpt_runner,
        mpt_signal: sender,
        identity,
        metadata,
        complete_receiver,
        l1_node,
        mints,
        chain_provider,
        oracel,
        soon_path: temp,
    })
}

/// promote_multi_tx will promote more than 200 random blocks
/// including modify `data` and `lamports` of all accounts.
pub async fn multi_l2_tx_to_execution(
    env: &mut E2EKailuaSoonEnvironment,
) -> Result<Vec<Arc<Execution>>> {
    let blocks = 50usize;
    let mut executions = vec![];
    let mut executor = env.e2e_producer.get_executor().clone();
    let last_blockhash = executor.storage_query(|s| Ok(s.current_bank().last_blockhash()))?;

    let mut boot_info = BootInfo {
        l1_head: B256::ZERO,
        agreed_l2_output_root: B256::ZERO,
        agreed_l2_block_number: 0,
        claimed_l2_output_root: B256::ZERO,
        claimed_l2_block_number: blocks as u64 + 1,
        rollup_config: SoonRollupConfig {
            sequencer_schedules: vec![(0, env.identity.pubkey())],
            ..Default::default()
        },
        chain_id: 0,
    };
    // firstly we need load prepared slot1.
    let (block_1, block_info) = get_l2_block_by_executor(&mut executor, 1).await?;
    let execution = to_execution(block_1, B256::ZERO, B256::ZERO, block_info)?;
    executions.push(Arc::new(execution));

    for i in 0..blocks {
        let slot = (i + 2) as u64;
        let from = env.mints[i % env.mints.len()].insecure_clone();
        let to = env.mints[(i + 1) % env.mints.len()].pubkey();
        let tx = system_transaction::transfer(&from, &to, LAMPORTS_PER_SOL, last_blockhash);
        env.e2e_producer.add_tx(tx)?;
        env.e2e_producer.mine_with_block(None)?;
        env.complete_receiver.try_recv()?;
        // finalize at once.
        executor.finalize(slot - 1)?;
        let (block, header) = get_l2_block_by_executor(&mut executor, slot).await?;
        let execution = to_execution(block, B256::ZERO, B256::ZERO, header)?;
        executions.push(Arc::new(execution));
    }
    Ok(executions)
}

pub async fn get_l2_block_by_executor(
    executor: &mut SharedExecutor,
    slot: u64,
) -> Result<(L2Block, L2BlockInfo)> {
    let block = executor.block_by_number(slot).await?;
    info!("finish block {:?}", block);
    let info = executor.l2_block_info_by_number(slot).await?;
    Ok((block, info))
}

pub mod hints {
    use super::*;
    use anyhow::Ok;
    use async_trait::async_trait;
    use solana_sdk::account::{AccountSharedData, WritableAccount};
    use soon_mpt_primitives::constants::KECCAK_EMPTY;
    use soon_primitives::{
        mpt::{AccountWithTrie, WrappedSolanaAccount},
        output_root::OutputRoot,
    };
    use std::sync::Arc;

    use kona_host::{HintHandler, OnlineHostBackendCfg, SharedKeyValueStore};
    use kona_proof::{Hint, HintType};
    use soon_node::{derive::driver::L2ChainProviderImmutable, executor::SharedExecutor};

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

use kona_executor::TrieDBProvider;
use kona_mpt::{TrieHinter, TrieProvider};
use kona_preimage::{HintWriter, NativeChannel, OracleReader};
use kona_proof::{errors::OracleProviderError, HintType};

#[derive(thiserror::Error, Debug, Eq, PartialEq)]
#[error("TestE2EError: {0}")]
pub struct TestE2EError(&'static str);

#[derive(Clone)]
pub struct E2EOracle {
    pub hint_writer: HintWriter<NativeChannel>,
    pub preimage_reader: OracleReader<NativeChannel>,
}

impl TrieProvider for E2EOracle {
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

impl TrieDBProvider for E2EOracle {
    fn data_by_hash(
        &self,
        code_hash: alloy_primitives::B256,
    ) -> Result<alloy_primitives::Bytes, Self::Error> {
        unimplemented!("data by hash")
    }
}

impl TrieHinter for E2EOracle {
    type Error = OracleProviderError;

    fn hint_trie_node(&self, hash: alloy_primitives::B256) -> Result<(), Self::Error> {
        kona_proof::block_on(async move {
            HintType::L2StateNode
                .with_data(&[hash.as_slice()])
                // .with_data(
                //     self.chain_id
                //         .map_or_else(Vec::new, |id| id.to_be_bytes().to_vec()),
                // )
                .send(&self.hint_writer)
                .await
        })
    }

    fn hint_account_proof(
        &self,
        pubkey: &solana_sdk::pubkey::Pubkey,
        block_number: u64,
    ) -> Result<(), Self::Error> {
        kona_proof::block_on(async move {
            tracing::info!("hint_account_proof, pubkey: {:?}", pubkey);
            let hashed_address = keccak256(pubkey.as_ref());
            tracing::info!("hint_account_proof, hashed_address: {:?}", hashed_address);
            HintType::L2AccountProof
                .with_data(&[block_number.to_be_bytes().as_ref(), hashed_address.as_ref()])
                .send(&self.hint_writer)
                .await
        })
    }

    fn hint_bank_hash(&self, block_number: u64) -> Result<(), Self::Error> {
        todo!()
    }

    fn hint_block_time(&self, block_number: u64) -> Result<(), Self::Error> {
        todo!()
    }
}

#[async_trait::async_trait]
impl PreimageOracleClient for E2EOracle {
    async fn get(&self, key: PreimageKey) -> PreimageOracleResult<Vec<u8>> {
        self.preimage_reader.get(key).await
    }

    async fn get_exact(&self, key: PreimageKey, buf: &mut [u8]) -> PreimageOracleResult<()> {
        self.preimage_reader.get_exact(key, buf).await
    }
}

#[async_trait::async_trait]
impl HintWriterClient for E2EOracle {
    async fn write(&self, hint: &str) -> PreimageOracleResult<()> {
        self.hint_writer.write(hint).await
    }
}

pub fn run_l2_core_client<
    E,
    O: CommsClient + Send + Sync,
    L2: L2ChainProvider + Send + Sync,
    T: TrieDBProvider + TrieHinter + Clone + Send + Sync,
>(
    oracle: O,
    mut l2_provider: L2,
    mut trie_provider: T,
    execution_cache: Vec<Arc<Execution>>,
) -> anyhow::Result<(BootInfo, B256)>
where
    E: L2BlockBuilder<T, T> + Send + Sync,
    L2: L2ChainProvider<Error = soon_node::Error>,
{
    let (boot_info, output_root) = kona_proof::block_on(async move {
        client::log("BOOT");
        let boot = BootInfo::load(&oracle).await.context("BootInfo::load")?;
        let rollup_config = Arc::new(boot.rollup_config.clone());

        client::log("SAFE HEAD");
        let safe_head = l2_provider
            .l2_block_info_by_number(boot.agreed_l2_block_number)
            .await?;
        let safe_head_header = L2BlockHeader {
            block_info: safe_head.block_info,
            account_root: B256::ZERO,
            widthdraw_root: B256::ZERO,
        };
        client::log("SAFE HEAD done");

        if boot.claimed_l2_block_number < safe_head_header.block_info.number {
            bail!("Invalid claim");
        }
        let safe_head_number = safe_head_header.block_info.number;
        info!(
            "SAFE HEAD number: {}, claimed_l2_block_number: {}",
            safe_head_number, boot.claimed_l2_block_number
        );
        let expected_output_count = (boot.claimed_l2_block_number - safe_head_number) as usize;

        let mut kona_executor = KonaExecutor::<_, _, E>::new(
            rollup_config.clone(),
            trie_provider.clone(),
            trie_provider.clone(),
            None,
        );
        kona_executor.update_safe_head(safe_head_header)?;

        let mut latest_output_root = boot.agreed_l2_output_root;
        for execution in execution_cache {
            info!(
                "enter execution {}/{}",
                execution.artifacts.block_info.block_info.number, boot.claimed_l2_block_number
            );
            let executor_result = kona_executor
                .execute_payload(execution.attributes.clone())
                .await?;
            latest_output_root = kona_executor
                .compute_output_root()
                .context("compute_output_root: Verify post state")?;
            kona_executor.update_safe_head(L2BlockHeader {
                block_info: execution.artifacts.block_info.block_info,
                account_root: executor_result.state_root,
                widthdraw_root: executor_result.withdraw_root,
            })?;
            // Verify post state
            assert_eq!(execution.claimed_output, latest_output_root);
            client::log(&format!(
                "OUTPUT: {}/{}",
                execution.artifacts.block_info.block_info.number, boot.claimed_l2_block_number
            ));
        }
        Ok((boot, latest_output_root))
    })?;
    return Ok((boot_info, output_root));
}
