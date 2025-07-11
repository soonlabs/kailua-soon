use anyhow::Result;
use kailua_common::{
    oracle::offline::{OfflineKeyValueStore, OfflineOracle},
    precondition::PreconditionValidationData,
};
use soon_primitives::rollup_config::SoonRollupConfig;
use kona_proof::BootInfo;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
use tracing::{debug, info};

mod client;

pub use client::OfflineClient;
pub type OfflineOracleShared = Arc<OfflineOracle<OfflineKeyValueStore>>;

// Custom deserializer for BootInfo that handles missing or incomplete rollup_config
fn deserialize_boot_info_with_default_rollup_config<'de, D>(
    deserializer: D,
) -> Result<BootInfo, D::Error>
where
    D: Deserializer<'de>,
{
    let mut value: Value = Deserialize::deserialize(deserializer)?;

    // Check if rollup_config is missing, empty, or incomplete
    if let Some(rollup_config) = value.get_mut("rollup_config") {
        if rollup_config.is_null() || rollup_config == &Value::Object(serde_json::Map::new()) {
            debug!("rollup_config is empty or null, using default");
            *rollup_config =
                serde_json::to_value(SoonRollupConfig::default()).map_err(serde::de::Error::custom)?;
        } else {
            // Check if it's missing required fields like genesis
            if rollup_config.get("genesis").is_none() {
                debug!("rollup_config missing genesis field, using default");
                *rollup_config = serde_json::to_value(SoonRollupConfig::default())
                    .map_err(serde::de::Error::custom)?;
            }
        }
    } else {
        debug!("rollup_config field missing, using default");
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "rollup_config".to_string(),
                serde_json::to_value(SoonRollupConfig::default()).map_err(serde::de::Error::custom)?,
            );
        }
    }

    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineConfig {
    #[serde(deserialize_with = "deserialize_boot_info_with_default_rollup_config")]
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
        let config_content = std::fs::read_to_string(cfg_path)?;
        let cfg: Self = serde_json::from_str(&config_content)?;
        info!("Loaded config: {:?}", cfg);
        Ok(cfg)
    }
}
