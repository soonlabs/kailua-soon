use crate::oracle::WitnessOracle;
use async_trait::async_trait;
use kona_preimage::errors::{PreimageOracleError, PreimageOracleResult};
use kona_preimage::{HintWriterClient, PreimageKey, PreimageOracleClient};
use kona_proof::boot::*;
use kona_proof::{BootInfo, FlushableCache};
use std::collections::HashMap;
use std::fmt::Debug;
use tracing::{debug, info, warn};

#[derive(Debug)]
pub struct MockOracle {
    pub map: HashMap<PreimageKey, Vec<u8>>,
}

impl Default for MockOracle {
    fn default() -> Self {
        Self::new(BootInfo {
            l1_head: Default::default(),
            agreed_l2_output_root: Default::default(),
            claimed_l2_output_root: Default::default(),
            claimed_l2_block_number: 0,
            agreed_l2_block_number: 0,
            chain_id: 0,
            rollup_config: Default::default(),
        })
    }
}

impl WitnessOracle for MockOracle {
    fn preimage_count(&self) -> usize {
        self.map.len()
    }

    fn validate_preimages(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn insert_preimage(&mut self, key: PreimageKey, value: Vec<u8>) {
        debug!(
            "inserting preimage: {}, value: {}",
            key,
            hex::encode(&value)
        );
        self.map.insert(key, value);
    }

    fn finalize_preimages(&mut self, _shard_size: usize, _with_validation_cache: bool) {
        info!("mock oracle finalize_preimages");
    }
}

impl Clone for MockOracle {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}

impl MockOracle {
    pub fn new(boot_info: BootInfo) -> Self {
        let mut oracle = Self {
            map: Default::default(),
        };
        Self::save_boot_info(&boot_info, &mut oracle);
        oracle
    }

    fn get_value(&self, key: PreimageKey) -> PreimageOracleResult<&Vec<u8>> {
        self.map.get(&key).ok_or_else(|| {
            warn!("mock oracle get value {} not found", key);
            PreimageOracleError::KeyNotFound
        })
    }

    fn save_boot_info(boot_info: &BootInfo, oracle: &mut MockOracle) {
        oracle.insert_preimage(
            PreimageKey::new_local(L1_HEAD_KEY.to()),
            boot_info.l1_head.to_vec(),
        );
        oracle.insert_preimage(
            PreimageKey::new_local(L2_OUTPUT_ROOT_KEY.to()),
            boot_info.agreed_l2_output_root.to_vec(),
        );
        oracle.insert_preimage(
            PreimageKey::new_local(L2_CLAIM_KEY.to()),
            boot_info.claimed_l2_output_root.to_vec(),
        );
        oracle.insert_preimage(
            PreimageKey::new_local(L2_CLAIM_BLOCK_NUMBER_KEY.to()),
            boot_info.claimed_l2_block_number.to_be_bytes().to_vec(),
        );
        oracle.insert_preimage(
            PreimageKey::new_local(L2_CHAIN_ID_KEY.to()),
            boot_info.chain_id.to_be_bytes().to_vec(),
        );
        oracle.insert_preimage(
            PreimageKey::new_local(L2_AGREED_BLOCK_NUMBER_KEY.to()),
            boot_info.agreed_l2_block_number.to_be_bytes().to_vec(),
        );
    }
}

impl FlushableCache for MockOracle {
    fn flush(&self) {
        info!("mock oracle flush");
    }
}

#[async_trait]
impl PreimageOracleClient for MockOracle {
    async fn get(&self, key: PreimageKey) -> PreimageOracleResult<Vec<u8>> {
        self.get_value(key).map(|v| {
            debug!("mock oracle get {} value: {}", key, hex::encode(v));
            v.clone()
        })
    }

    async fn get_exact(&self, key: PreimageKey, buf: &mut [u8]) -> PreimageOracleResult<()> {
        let value = self.get_value(key)?;
        debug!(
            "mock oracle get exact {} value: {}",
            key,
            hex::encode(value)
        );
        buf.copy_from_slice(value);
        Ok(())
    }
}

#[async_trait]
impl HintWriterClient for MockOracle {
    async fn write(&self, hint: &str) -> PreimageOracleResult<()> {
        info!("mock oracle write: {:?}", hint);
        Ok(())
    }
}
