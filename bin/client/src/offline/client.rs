use alloy_primitives::{Address, B256};
use kailua_common::client::stitching::run_stitching_client;
use kona_proof::{executor::OffchainL2Builder, l1::OracleBlobProvider};
use tracing::info;

use crate::offline::{OfflineConfig, OfflineOracleShared};

pub struct OfflineClient {
    oracle: OfflineOracleShared,
    cfg: OfflineConfig,
}

impl OfflineClient {
    pub fn new(oracle: OfflineOracleShared, cfg: OfflineConfig) -> Self {
        Self { oracle, cfg }
    }

    pub fn run(&self) {
        if self.cfg.native_client {
            self.run_native();
        } else {
            self.run_zkvm();
        }
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
