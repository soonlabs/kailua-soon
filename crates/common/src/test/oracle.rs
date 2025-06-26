use alloy_primitives::B256;
use async_trait::async_trait;
use kona_host::KeyValueStore;
use kona_preimage::errors::PreimageOracleResult;
use kona_preimage::{HintWriterClient, PreimageKey, PreimageOracleClient};
use kona_proof::{BootInfo, FlushableCache};
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

use crate::oracle::offline::{OfflineKeyValueStore, OfflineOracle};
use crate::oracle::WitnessOracle;
use crate::precondition::PreconditionValidationData;

#[derive(Debug)]
pub struct TestOracle<T: KeyValueStore + Send + Sync + Debug> {
    pub inner: OfflineOracle<T>,
    pub temp_dir: Option<TempDir>,
}

impl Default for TestOracle<OfflineKeyValueStore> {
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

impl WitnessOracle for TestOracle<OfflineKeyValueStore> {
    fn preimage_count(&self) -> usize {
        self.inner.preimage_count()
    }

    fn validate_preimages(&self) -> anyhow::Result<()> {
        self.inner.validate_preimages()
    }

    fn insert_preimage(&mut self, key: PreimageKey, value: Vec<u8>) {
        self.inner.insert_preimage(key, value)
    }

    fn finalize_preimages(&mut self, shard_size: usize, with_validation_cache: bool) {
        self.inner
            .finalize_preimages(shard_size, with_validation_cache)
    }
}

impl<T: KeyValueStore + Send + Sync + Debug> Clone for TestOracle<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            temp_dir: None,
        }
    }
}

impl TestOracle<OfflineKeyValueStore> {
    pub fn new(boot_info: BootInfo) -> Self {
        // Create a cloned disk store in a temp dir
        let temp_dir = tempdir().unwrap();
        Self::new_with_path(
            boot_info,
            concat!(env!("CARGO_MANIFEST_DIR"), "/testdata").into(),
            temp_dir.path().to_path_buf(),
            Some(temp_dir),
        )
    }

    pub fn new_with_path(
        boot_info: BootInfo,
        source_db_path: PathBuf,
        target_db_path: PathBuf,
        temp_dir: Option<TempDir>,
    ) -> Self {
        Self {
            inner: OfflineOracle::new(boot_info, source_db_path, Some(target_db_path)),
            temp_dir,
        }
    }

    pub fn add_precondition_data(&self, data: PreconditionValidationData) -> B256 {
        self.inner.add_precondition_data(data)
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
        self.inner.get(key).await
    }

    async fn get_exact(&self, key: PreimageKey, buf: &mut [u8]) -> PreimageOracleResult<()> {
        self.inner.get_exact(key, buf).await
    }
}

#[async_trait]
impl<T: KeyValueStore + Send + Sync + Debug> HintWriterClient for TestOracle<T> {
    async fn write(&self, _hint: &str) -> PreimageOracleResult<()> {
        self.inner.write(_hint).await
    }
}

/// Create a TestOracle with data access analysis enabled
pub fn create_analyzed_oracle(boot_info: BootInfo) -> Arc<TestOracle<OfflineKeyValueStore>> {
    let mut oracle = TestOracle::new(boot_info);
    oracle.inner.enable_analysis();
    Arc::new(oracle)
}

#[cfg(test)]
mod tests {
    use crate::oracle::offline::AccessAnalyzer;

    use super::*;
    use alloy_primitives::b256;
    use kona_preimage::PreimageKeyType;
    use kona_proof::BootInfo;

    #[test]
    fn test_oracle_analysis_enable_disable() {
        let boot_info = BootInfo {
            l1_head: b256!("0x417ffee9dd1ccbd35755770dd8c73dbdcd96ba843c532788850465bdd08ea495"),
            agreed_l2_output_root: b256!(
                "0x82da7204148ba4d8d59e587b6b3fdde5561dc31d9e726220f7974bf9f2158d75"
            ),
            claimed_l2_output_root: b256!(
                "0x6984e5ae4d025562c8a571949b985692d80e364ddab46d5c8af5b36a20f611d1"
            ),
            claimed_l2_block_number: 16491349,
            chain_id: 11155420,
            rollup_config: Default::default(),
        };

        // Test creating oracle without analysis
        let mut oracle = TestOracle::new(boot_info.clone());
        assert!(!oracle.inner.is_analysis_enabled());
        assert!(oracle.inner.get_stats().is_none());

        // Test enabling analysis
        oracle.inner.enable_analysis();
        assert!(oracle.inner.is_analysis_enabled());
        let stats = oracle.inner.get_stats().unwrap();
        assert_eq!(stats.total_accesses, 0);

        // Test creating oracle with analysis enabled from start
        let oracle_with_analysis = create_analyzed_oracle(boot_info.clone());
        assert!(oracle_with_analysis.inner.is_analysis_enabled());

        // Test disabling analysis
        oracle.inner.disable_analysis();
        assert!(!oracle.inner.is_analysis_enabled());
        assert!(oracle.inner.get_stats().is_none());
    }

    #[test]
    fn test_access_analyzer_functionality() {
        let analyzer = AccessAnalyzer::new();

        let key1 = PreimageKey::new([1u8; 32], PreimageKeyType::Keccak256);
        let key2 = PreimageKey::new([2u8; 32], PreimageKeyType::Sha256);

        analyzer.record_access(key1, 100);
        analyzer.record_access(key2, 200);

        let stats = analyzer.get_stats();
        assert_eq!(stats.total_accesses, 2);
        assert_eq!(stats.key_type_counts.get("Keccak256"), Some(&1));
        assert_eq!(stats.key_type_counts.get("Sha256"), Some(&1));
        assert_eq!(stats.data_sizes.get("Keccak256"), Some(&100));
        assert_eq!(stats.data_sizes.get("Sha256"), Some(&200));

        // Test clear functionality
        analyzer.clear();
        let cleared_stats = analyzer.get_stats();
        assert_eq!(cleared_stats.total_accesses, 0);
        assert!(cleared_stats.key_type_counts.is_empty());
    }

    #[test]
    fn test_create_analyzed_oracle() {
        let boot_info = BootInfo {
            l1_head: b256!("0x417ffee9dd1ccbd35755770dd8c73dbdcd96ba843c532788850465bdd08ea495"),
            agreed_l2_output_root: b256!(
                "0x82da7204148ba4d8d59e587b6b3fdde5561dc31d9e726220f7974bf9f2158d75"
            ),
            claimed_l2_output_root: b256!(
                "0x6984e5ae4d025562c8a571949b985692d80e364ddab46d5c8af5b36a20f611d1"
            ),
            claimed_l2_block_number: 16491349,
            chain_id: 11155420,
            rollup_config: Default::default(),
        };

        let oracle = create_analyzed_oracle(boot_info);
        assert!(oracle.inner.is_analysis_enabled());
        assert_eq!(oracle.inner.get_stats().unwrap().total_accesses, 0);
    }
}
