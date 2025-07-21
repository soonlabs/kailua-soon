use crate::offline::{OfflineClient, OfflineConfig};
use alloy_primitives::{Address, B256};
use anyhow::Result;
use kailua_common::{client::stitching::run_stitching_client, test::mock::MockOracle};
use kona_proof::{executor::OffchainL2Builder, l1::OracleBlobProvider};
use std::sync::Arc;
use tracing::{error, info};

pub type MockOracleShared = Arc<MockOracle>;

pub struct OffchainClient {
    oracle: MockOracleShared,
    cfg: OfflineConfig,
}

impl OffchainClient {
    pub fn new(cfg: OfflineConfig) -> Result<Self> {
        let oracle = MockOracle::new_with_path(
            cfg.boot_info.clone(),
            cfg.source_db_path.clone(),
            cfg.target_db_path.clone(),
        )?;
        // if cfg.analysis {
        //     oracle.enable_analysis();
        // }

        Ok(Self {
            oracle: Arc::new(oracle),
            cfg,
        })
    }

    pub fn run_native(&self) {
        let precondition_validation_data_hash = self
            .cfg
            .precondition_validation_data
            .as_ref()
            .map_or_else(|| B256::ZERO, |data| data.hash());
        let proof_journal = run_stitching_client::<OffchainL2Builder<_, _>, _, _>(
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

impl OfflineClient for OffchainClient {
    fn run(&self) {
        if self.cfg.native_client {
            self.run_native();
        } else {
            self.run_zkvm();
        }
    }
}
