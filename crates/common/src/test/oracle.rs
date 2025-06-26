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
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::{tempdir, TempDir};
use tokio::sync::RwLock;
use tokio::task::block_in_place;

use crate::oracle::WitnessOracle;
use crate::precondition::PreconditionValidationData;

/// Data access statistics
#[derive(Debug, Clone, Default)]
pub struct AccessStats {
    pub total_accesses: usize,
    pub key_type_counts: HashMap<String, usize>,
    pub data_sizes: HashMap<String, usize>,
    pub accessed_keys: Vec<(PreimageKey, usize)>,
}

/// Access pattern analyzer
#[derive(Debug, Clone, Default)]
pub struct AccessAnalyzer {
    stats: Arc<Mutex<AccessStats>>,
}

impl AccessAnalyzer {
    pub fn new() -> Self {
        Self {
            stats: Arc::new(Mutex::new(AccessStats::default())),
        }
    }

    /// Record data access
    pub fn record_access(&self, key: PreimageKey, data_size: usize) {
        let mut stats = self.stats.lock().unwrap();

        stats.total_accesses += 1;
        stats.accessed_keys.push((key, data_size));

        let key_type = format!("{:?}", key.key_type());
        *stats.key_type_counts.entry(key_type.clone()).or_insert(0) += 1;
        *stats.data_sizes.entry(key_type).or_insert(0) += data_size;
    }

    /// Get statistics
    pub fn get_stats(&self) -> AccessStats {
        self.stats.lock().unwrap().clone()
    }

    /// Print detailed statistics
    pub fn print_analysis(&self, test_name: &str) {
        let stats = self.get_stats();

        println!("\n=== {} Data Access Analysis ===", test_name);
        println!("Total accesses: {}", stats.total_accesses);

        if stats.total_accesses == 0 {
            println!("No data access detected");
            return;
        }

        println!("\nStatistics by preimage type:");
        println!(
            "{:<20} {:<10} {:<15} {:<12}",
            "Key Type", "Accesses", "Total Size(bytes)", "Average Size"
        );
        println!("{:-<60}", "");

        for (key_type, &count) in &stats.key_type_counts {
            let total_size = stats.data_sizes.get(key_type).unwrap_or(&0);
            let avg_size = if count > 0 { total_size / count } else { 0 };
            println!(
                "{:<20} {:<10} {:<15} {:<12}",
                key_type, count, total_size, avg_size
            );
        }

        // Show specific accessed keys (first 10)
        println!("\nAccessed preimage keys (first 10):");
        for (i, (key, size)) in stats.accessed_keys.iter().take(10).enumerate() {
            let key_bytes = format!("{:?}", key);
            let display_key = if key_bytes.len() > 50 {
                format!(
                    "{}...{}",
                    &key_bytes[0..25],
                    &key_bytes[key_bytes.len() - 20..]
                )
            } else {
                key_bytes
            };
            println!(
                "{}. {:?}: {} ({} bytes)",
                i + 1,
                key.key_type(),
                display_key,
                size
            );
        }

        if stats.accessed_keys.len() > 10 {
            println!("... {} more access records", stats.accessed_keys.len() - 10);
        }
    }

    /// Clear statistics
    pub fn clear(&self) {
        *self.stats.lock().unwrap() = AccessStats::default();
    }
}

#[derive(Debug)]
pub struct TestOracle<T: KeyValueStore + Send + Sync + Debug> {
    pub kv: Arc<RwLock<T>>,
    pub backend: OfflineHostBackend<T>,
    pub temp_dir: Option<TempDir>,
    /// Access analyzer (optional, for data access analysis)
    pub analyzer: Option<AccessAnalyzer>,
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
            analyzer: self.analyzer.clone(),
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

    /// Create a new TestOracle with data access analysis enabled
    pub fn new_with_analysis(boot_info: BootInfo) -> Self {
        let mut oracle = Self::new(boot_info);
        oracle.analyzer = Some(AccessAnalyzer::new());
        oracle
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
            analyzer: None,
        }
    }

    /// Create a new TestOracle with analysis enabled and custom path
    pub fn new_with_path_and_analysis(
        boot_info: BootInfo,
        source_db_path: PathBuf,
        target_db_path: PathBuf,
    ) -> Self {
        let mut oracle = Self::new_with_path(boot_info, source_db_path, target_db_path);
        oracle.analyzer = Some(AccessAnalyzer::new());
        oracle
    }

    /// Enable data access analysis
    pub fn enable_analysis(&mut self) {
        self.analyzer = Some(AccessAnalyzer::new());
    }

    /// Disable data access analysis
    pub fn disable_analysis(&mut self) {
        self.analyzer = None;
    }

    /// Check if analysis is enabled
    pub fn is_analysis_enabled(&self) -> bool {
        self.analyzer.is_some()
    }

    /// Get access statistics (if analysis is enabled)
    pub fn get_stats(&self) -> Option<AccessStats> {
        self.analyzer.as_ref().map(|a| a.get_stats())
    }

    /// Print analysis results (if analysis is enabled)
    pub fn print_analysis(&self, test_name: &str) {
        if let Some(analyzer) = &self.analyzer {
            analyzer.print_analysis(test_name);
        } else {
            println!("Data access analysis is not enabled");
        }
    }

    /// Clear statistics (if analysis is enabled)
    pub fn clear_stats(&self) {
        if let Some(analyzer) = &self.analyzer {
            analyzer.clear();
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
        let result = self.backend.get_preimage(key).await;

        // Record access if analysis is enabled
        if let (Ok(ref data), Some(analyzer)) = (&result, &self.analyzer) {
            analyzer.record_access(key, data.len());
        }

        result
    }

    async fn get_exact(&self, key: PreimageKey, buf: &mut [u8]) -> PreimageOracleResult<()> {
        let result = self.backend.get_preimage(key).await;

        match result {
            Ok(value) => {
                buf.copy_from_slice(value.as_ref());

                // Record access if analysis is enabled
                if let Some(analyzer) = &self.analyzer {
                    analyzer.record_access(key, buf.len());
                }

                Ok(())
            }
            Err(e) => Err(e),
        }
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

/// Create a TestOracle with data access analysis enabled
pub fn create_analyzed_oracle(boot_info: BootInfo) -> Arc<TestOracle<TestKeyValueStore>> {
    Arc::new(TestOracle::new_with_analysis(boot_info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::b256;
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
        assert!(!oracle.is_analysis_enabled());
        assert!(oracle.get_stats().is_none());

        // Test enabling analysis
        oracle.enable_analysis();
        assert!(oracle.is_analysis_enabled());
        let stats = oracle.get_stats().unwrap();
        assert_eq!(stats.total_accesses, 0);

        // Test creating oracle with analysis enabled from start
        let oracle_with_analysis = TestOracle::new_with_analysis(boot_info.clone());
        assert!(oracle_with_analysis.is_analysis_enabled());

        // Test disabling analysis
        oracle.disable_analysis();
        assert!(!oracle.is_analysis_enabled());
        assert!(oracle.get_stats().is_none());
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
        assert!(oracle.is_analysis_enabled());
        assert_eq!(oracle.get_stats().unwrap().total_accesses, 0);
    }
}
