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

        // Convert stored executions to the format expected by run_stitching_client
        // stitched_executions is Vec<Vec<Execution>>, where each inner Vec represents a batch of executions
        let stitched_executions = if self.oracle.executions.is_empty() {
            info!("No executions found in oracle");
            vec![]
        } else {
            info!(
                "Found {} executions in oracle, converting to stitched format",
                self.oracle.executions.len()
            );
            // All executions in a single batch
            vec![self
                .oracle
                .executions
                .iter()
                .map(|e| e.as_ref().clone())
                .collect()]
        };

        let proof_journal = run_stitching_client::<OffchainL2Builder<_, _>, _, _>(
            precondition_validation_data_hash,
            self.oracle.clone(),
            self.oracle.clone(),
            OracleBlobProvider::new(self.oracle.clone()),
            B256::ZERO,
            Address::ZERO,
            stitched_executions,
            vec![],
        );
        assert_eq!(
            self.cfg.boot_info.agreed_l2_output_root,
            proof_journal.agreed_l2_output_root
        );
        assert_eq!(
            self.cfg.boot_info.claimed_l2_block_number,
            proof_journal.claimed_l2_block_number
        );
        assert_eq!(
            self.cfg.boot_info.claimed_l2_output_root,
            proof_journal.claimed_l2_output_root
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
