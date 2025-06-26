use anyhow::Result;
use kailua_common::{
    oracle::offline::{OfflineKeyValueStore, OfflineOracle},
    precondition::PreconditionValidationData,
};
use kona_proof::BootInfo;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

mod client;

pub use client::OfflineClient;
pub type OfflineOracleShared = Arc<OfflineOracle<OfflineKeyValueStore>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineConfig {
    pub boot_info: BootInfo,
    pub source_db_path: PathBuf,
    pub target_db_path: Option<PathBuf>,
    pub precondition_validation_data: Option<PreconditionValidationData>,
    pub analysis: bool,
    pub native_client: bool,
}

pub fn run_offline_client(cfg_path: PathBuf) -> Result<()> {
    let cfg = OfflineConfig::load(cfg_path)?;
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
    let client = OfflineClient::new(Arc::new(oracle), cfg);
    client.run();
    Ok(())
}

impl OfflineConfig {
    pub fn load(cfg_path: PathBuf) -> Result<Self> {
        let cfg = serde_json::from_str(&std::fs::read_to_string(cfg_path)?)?;
        Ok(cfg)
    }
}
