use crate::offline::{OfflineClient, OfflineConfig};
use alloy_primitives::{Address, B256};
use kailua_common::{
    client::stitching::run_stitching_client,
    oracle::offline::{OfflineKeyValueStore, OfflineOracle},
};
use kona_proof::{executor::StatelessL2Builder, l1::OracleBlobProvider};
use std::sync::Arc;
use tracing::info;

pub type OfflineOracleShared = Arc<OfflineOracle<OfflineKeyValueStore>>;

pub struct StatelessClient {
    oracle: OfflineOracleShared,
    cfg: OfflineConfig,
}

impl StatelessClient {
    pub fn new(cfg: OfflineConfig) -> Self {
        let mut oracle = OfflineOracle::new(
            cfg.boot_info.clone(),
            cfg.source_db_path.clone(),
            cfg.target_db_path.clone(),
        );
        if cfg.analysis {
            oracle.enable_analysis();
        }
        if let Some(precondition_validation_data) = cfg.precondition_validation_data.as_ref() {
            oracle.add_precondition_data(precondition_validation_data.clone());
        }
        Self {
            oracle: Arc::new(oracle),
            cfg,
        }
    }

    pub fn run_native(&self) {
        let precondition_validation_data_hash = self
            .cfg
            .precondition_validation_data
            .as_ref()
            .map_or_else(|| B256::ZERO, |data| data.hash());
        let proof_journal = run_stitching_client::<StatelessL2Builder<_, _>, _, _>(
            precondition_validation_data_hash,
            self.oracle.clone(),
            self.oracle.clone(),
            OracleBlobProvider::new(self.oracle.clone()),
            B256::ZERO,
            Address::ZERO,
            vec![],
            vec![],
        );
        info!("Proof journal: {:?}", proof_journal);
    }

    pub fn run_zkvm(&self) {}
}

impl OfflineClient for StatelessClient {
    fn run(&self) {
        if self.cfg.native_client {
            self.run_native();
        } else {
            self.run_zkvm();
        }
    }
}
