use alloy_primitives::B256;
use async_trait::async_trait;
use copy_dir::copy_dir;
use kona_host::single::{SingleChainHost, SingleChainLocalInputs};
use kona_host::{DiskKeyValueStore, KeyValueStore, OfflineHostBackend, SplitKeyValueStore};
use kona_preimage::errors::PreimageOracleResult;
use kona_preimage::{
    HintWriterClient, PreimageFetcher, PreimageKey, PreimageKeyType, PreimageOracleClient,
};
use kona_proof::{BootInfo, FlushableCache};
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tokio::sync::RwLock;
use tokio::task::block_in_place;

use crate::oracle::WitnessOracle;
use crate::precondition::PreconditionValidationData;

#[derive(Debug)]
pub struct TestOracle<T: KeyValueStore + Send + Sync + Debug> {
    pub kv: Arc<RwLock<T>>,
    pub backend: OfflineHostBackend<T>,
    pub temp_dir: Option<TempDir>,
}

impl Default for TestOracle<TestKeyValueStore> {
    fn default() -> Self {
        Self::new(BootInfo {
            l1_head: Default::default(),
            agreed_l2_output_root: Default::default(),
            claimed_l2_output_root: Default::default(),
            claimed_l2_block_number: 0,
            chain_id: 0,
            rollup_config: Default::default(),
        })
    }
}

impl WitnessOracle for TestOracle<TestKeyValueStore> {
    fn preimage_count(&self) -> usize {
        1
    }

    fn validate_preimages(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn insert_preimage(&mut self, _key: PreimageKey, _value: Vec<u8>) {}

    fn finalize_preimages(&mut self, _shard_size: usize, _with_validation_cache: bool) {}
}

impl<T: KeyValueStore + Send + Sync + Debug> Clone for TestOracle<T> {
    fn clone(&self) -> Self {
        Self {
            kv: self.kv.clone(),
            backend: OfflineHostBackend::new(self.kv.clone()),
            temp_dir: None,
        }
    }
}

pub type TestKeyValueStore = SplitKeyValueStore<SingleChainLocalInputs, DiskKeyValueStore>;

impl TestOracle<TestKeyValueStore> {
    pub fn new(boot_info: BootInfo) -> Self {
        // Create a cloned disk store in a temp dir
        let temp_dir = tempdir().unwrap();
        Self::new_with_path(
            boot_info,
            concat!(env!("CARGO_MANIFEST_DIR"), "/testdata").into(),
            temp_dir.path().to_path_buf(),
        )
    }

    pub fn new_with_path(
        boot_info: BootInfo,
        source_db_path: PathBuf,
        target_db_path: PathBuf,
    ) -> Self {
        // Create memory store
        let scli = SingleChainLocalInputs::new(SingleChainHost {
            l1_head: boot_info.l1_head,
            agreed_l2_output_root: boot_info.agreed_l2_output_root,
            claimed_l2_output_root: boot_info.claimed_l2_output_root,
            claimed_l2_block_number: boot_info.claimed_l2_block_number,
            l2_chain_id: Some(boot_info.chain_id),
            // rollup_config_path: None, // no support for custom chains
            ..Default::default()
        });
        // Create a cloned disk store in a temp dir
        let dest = target_db_path.join("testdata");
        copy_dir(source_db_path, &dest).unwrap();
        let disk = DiskKeyValueStore::new(dest);
        let kv = Arc::new(RwLock::new(SplitKeyValueStore::new(scli, disk)));

        Self {
            kv: kv.clone(),
            backend: OfflineHostBackend::new(kv.clone()),
            temp_dir: None, // We don't store the temp_dir PathBuf as TempDir anymore
        }
    }

    pub fn add_precondition_data(&self, data: PreconditionValidationData) -> B256 {
        block_in_place(move || {
            let mut kv = self.kv.blocking_write();
            let precondition_data_hash = data.hash();
            let preimage_key = PreimageKey::new(precondition_data_hash.0, PreimageKeyType::Sha256);
            kv.set(B256::from(preimage_key), data.to_vec()).unwrap();
            // sanity check
            assert_eq!(kv.get(B256::from(preimage_key)).unwrap(), data.to_vec());
            precondition_data_hash
        })
    }
}

impl<T: KeyValueStore + Send + Sync + Debug> FlushableCache for TestOracle<T> {
    fn flush(&self) {
        // noop
    }
}

#[async_trait]
impl<T: KeyValueStore + Send + Sync + Debug> PreimageOracleClient for TestOracle<T> {
    async fn get(&self, key: PreimageKey) -> PreimageOracleResult<Vec<u8>> {
        self.backend.get_preimage(key).await
    }

    async fn get_exact(&self, key: PreimageKey, buf: &mut [u8]) -> PreimageOracleResult<()> {
        let value = self.get(key).await?;
        buf.copy_from_slice(value.as_ref());
        Ok(())
    }
}

#[async_trait]
impl<T: KeyValueStore + Send + Sync + Debug> HintWriterClient for TestOracle<T> {
    async fn write(&self, _hint: &str) -> PreimageOracleResult<()> {
        // just hit the noop
        self.flush();
        Ok(())
    }
}
