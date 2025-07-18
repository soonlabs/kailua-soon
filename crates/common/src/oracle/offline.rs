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
pub struct OfflineOracle<T: KeyValueStore + Send + Sync + Debug> {
    pub kv: Arc<RwLock<T>>,
    pub backend: OfflineHostBackend<T>,
    /// Access analyzer (optional, for data access analysis)
    pub analyzer: Option<AccessAnalyzer>,
}

impl Default for OfflineOracle<OfflineKeyValueStore> {
    fn default() -> Self {
        Self::new(
            BootInfo {
                l1_head: Default::default(),
                agreed_l2_output_root: Default::default(),
                claimed_l2_output_root: Default::default(),
                claimed_l2_block_number: 0,
                agreed_l2_block_number: 0,
                chain_id: 0,
                rollup_config: Default::default(),
            },
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata")),
            None,
        )
    }
}

impl WitnessOracle for OfflineOracle<OfflineKeyValueStore> {
    fn preimage_count(&self) -> usize {
        1
    }

    fn validate_preimages(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn insert_preimage(&mut self, _key: PreimageKey, _value: Vec<u8>) {}

    fn finalize_preimages(&mut self, _shard_size: usize, _with_validation_cache: bool) {}
}

impl<T: KeyValueStore + Send + Sync + Debug> Clone for OfflineOracle<T> {
    fn clone(&self) -> Self {
        Self {
            kv: self.kv.clone(),
            backend: OfflineHostBackend::new(self.kv.clone()),
            analyzer: self.analyzer.clone(),
        }
    }
}

pub type OfflineKeyValueStore = SplitKeyValueStore<SingleChainLocalInputs, DiskKeyValueStore>;

impl OfflineOracle<OfflineKeyValueStore> {
    pub fn new(
        boot_info: BootInfo,
        source_db_path: PathBuf,
        target_db_path: Option<PathBuf>,
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
        let dest = if let Some(target_db_path) = target_db_path {
            let dest = target_db_path.join("testdata");
            copy_dir(source_db_path.clone(), &dest).unwrap_or_else(|_| {
                panic!(
                    "Failed to copy testdata from {} to {}",
                    source_db_path.display(),
                    dest.display()
                )
            });
            dest
        } else {
            source_db_path
        };
        let disk = DiskKeyValueStore::new(dest);
        let kv = Arc::new(RwLock::new(SplitKeyValueStore::new(scli, disk)));

        Self {
            kv: kv.clone(),
            backend: OfflineHostBackend::new(kv.clone()),
            analyzer: None,
        }
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

impl<T: KeyValueStore + Send + Sync + Debug> FlushableCache for OfflineOracle<T> {
    fn flush(&self) {
        // noop
    }
}

#[async_trait]
impl<T: KeyValueStore + Send + Sync + Debug> PreimageOracleClient for OfflineOracle<T> {
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
impl<T: KeyValueStore + Send + Sync + Debug> HintWriterClient for OfflineOracle<T> {
    async fn write(&self, _hint: &str) -> PreimageOracleResult<()> {
        // just hit the noop
        self.flush();
        Ok(())
    }
}
