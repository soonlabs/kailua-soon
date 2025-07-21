use crate::oracle::WitnessOracle;
use alloy_primitives::B256;
use async_trait::async_trait;
use copy_dir::copy_dir;
use kona_preimage::errors::{PreimageOracleError, PreimageOracleResult};
use kona_preimage::{HintWriterClient, PreimageKey, PreimageOracleClient};
use kona_proof::boot::*;
use kona_proof::{BootInfo, FlushableCache};
use std::collections::HashMap;
use std::fmt::Debug;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

const FULL_PRINT_SIZE: usize = 64;
const SHORT_PRINT_SIZE: usize = 8;

/// 存储统计信息
#[derive(Debug, Clone)]
pub struct StorageStats {
    pub entry_count: usize,
    pub total_value_size: usize,
    pub estimated_memory_usage: usize,
}

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
        Self::print_value("inserting", key, &value);
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

    pub fn new_with_path(
        boot_info: BootInfo,
        source_db_path: PathBuf,
        target_db_path: Option<PathBuf>,
    ) -> Self {
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

        let mut oracle = Self::new(boot_info);
        if dest.exists() {
            if let Err(e) = oracle.read_from_disk(dest) {
                warn!("Failed to read from disk: {}", e);
            }
        }
        oracle
    }

    pub fn write_to_disk(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();

        // make sure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        // write version
        writer.write_all(b"MOCK_V01")?;

        // write entry count
        let count = self.map.len() as u32;
        writer.write_all(&count.to_le_bytes())?;

        // write each key-value pair
        for (key, value) in &self.map {
            // write key (directly convert to 32 bytes)
            let key_b256: B256 = (*key).into();
            writer.write_all(key_b256.as_slice())?;

            // write value length and data
            let value_len = value.len() as u32;
            writer.write_all(&value_len.to_le_bytes())?;
            writer.write_all(value)?;
        }

        writer.flush()?;
        info!("Saved {} preimages to {}", count, path.display());
        Ok(())
    }

    pub fn read_from_disk(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // read version
        let mut version = [0u8; 8];
        reader.read_exact(&mut version)?;
        if &version != b"MOCK_V01" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file format or version",
            ));
        }

        // read entry count
        let mut count_bytes = [0u8; 4];
        reader.read_exact(&mut count_bytes)?;
        let count = u32::from_le_bytes(count_bytes);

        self.map.clear();
        self.map.reserve(count as usize);

        // read each key-value pair
        for _ in 0..count {
            // read key (32 bytes)
            let mut key_bytes = [0u8; 32];
            reader.read_exact(&mut key_bytes)?;
            let key = PreimageKey::try_from(key_bytes)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid preimage key"))?;

            // read value
            let mut value_len_bytes = [0u8; 4];
            reader.read_exact(&mut value_len_bytes)?;
            let value_len = u32::from_le_bytes(value_len_bytes) as usize;

            let mut value = vec![0u8; value_len];
            reader.read_exact(&mut value)?;

            self.map.insert(key, value);
        }

        info!("Loaded {} preimages from {}", count, path.display());
        Ok(())
    }

    /// save to disk and return the number of entries stored
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<usize> {
        self.write_to_disk(path)?;
        Ok(self.map.len())
    }

    /// load from disk and return the number of entries loaded
    pub fn load(&mut self, path: impl AsRef<Path>) -> io::Result<usize> {
        self.read_from_disk(path)?;
        Ok(self.map.len())
    }

    /// get storage statistics
    pub fn storage_stats(&self) -> StorageStats {
        let total_value_size: usize = self.map.values().map(|v| v.len()).sum();
        let estimated_memory_usage = total_value_size
            + self.map.len()
                * (std::mem::size_of::<PreimageKey>() + std::mem::size_of::<Vec<u8>>());

        StorageStats {
            entry_count: self.map.len(),
            total_value_size,
            estimated_memory_usage,
        }
    }

    /// clear all data
    pub fn clear(&mut self) {
        self.map.clear();
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

    fn print_value(operation: &str, key: PreimageKey, value: &Vec<u8>) {
        if value.len() > FULL_PRINT_SIZE {
            debug!(
                "{} preimage: {}, value (len {}): {}..{}",
                operation,
                key,
                value.len(),
                hex::encode(&value[..SHORT_PRINT_SIZE]),
                hex::encode(&value[value.len() - SHORT_PRINT_SIZE..])
            );
        } else {
            debug!(
                "{} preimage: {}, value (len {}): {}",
                operation,
                key,
                value.len(),
                hex::encode(value)
            );
        }
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
            Self::print_value("get", key, v);
            v.clone()
        })
    }

    async fn get_exact(&self, key: PreimageKey, buf: &mut [u8]) -> PreimageOracleResult<()> {
        let value = self.get_value(key)?;
        Self::print_value("get exact", key, value);
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow;
    use kona_preimage::PreimageKeyType;
    use tempfile::tempdir;

    fn create_test_oracle() -> MockOracle {
        let boot_info = BootInfo {
            l1_head: Default::default(),
            agreed_l2_output_root: Default::default(),
            claimed_l2_output_root: Default::default(),
            claimed_l2_block_number: 0,
            agreed_l2_block_number: 0,
            chain_id: 1,
            rollup_config: Default::default(),
        };
        MockOracle::new(boot_info)
    }

    #[test]
    fn test_basic_operations() {
        let mut oracle = create_test_oracle();

        // verify initial state - boot info creates 6 entries
        assert_eq!(oracle.preimage_count(), 6);

        // test insert and get
        let key = PreimageKey::new_local(999); // 使用不冲突的键值
        let value = b"test_value".to_vec();

        oracle.insert_preimage(key, value.clone());
        assert_eq!(oracle.preimage_count(), 7); // 6 boot info + 1 inserted

        // test statistics
        let stats = oracle.storage_stats();
        assert_eq!(stats.entry_count, 7);
        assert!(stats.total_value_size > 0);
        assert!(stats.estimated_memory_usage > stats.total_value_size);
    }

    #[test]
    fn test_save_and_load() -> std::io::Result<()> {
        let temp_dir = tempdir()?;
        let file_path = temp_dir.path().join("test_oracle.db");

        // create and save data
        let mut oracle = create_test_oracle();
        let key1 = PreimageKey::new_local(1001);
        let key2 = PreimageKey::new_local(1002);
        let value1 = b"test_value_1".to_vec();
        let value2 = b"test_value_2".to_vec();

        oracle.insert_preimage(key1, value1.clone());
        oracle.insert_preimage(key2, value2.clone());

        let initial_count = oracle.preimage_count();
        assert_eq!(initial_count, 8); // 6 boot info + 2 inserted

        // save to disk
        let saved_count = oracle.save(&file_path)?;
        assert_eq!(saved_count, initial_count);
        assert!(file_path.exists());

        // create new oracle and load data
        let mut new_oracle = MockOracle {
            map: HashMap::new(),
        };

        let loaded_count = new_oracle.load(&file_path)?;
        assert_eq!(loaded_count, initial_count);
        assert_eq!(new_oracle.preimage_count(), initial_count);

        // verify loaded data correctness
        assert!(new_oracle.map.contains_key(&key1));
        assert!(new_oracle.map.contains_key(&key2));
        assert_eq!(new_oracle.map.get(&key1).unwrap(), &value1);
        assert_eq!(new_oracle.map.get(&key2).unwrap(), &value2);

        Ok(())
    }

    #[test]
    fn test_write_to_disk_and_read_from_disk() -> std::io::Result<()> {
        let temp_dir = tempdir()?;
        let file_path = temp_dir.path().join("oracle.db");

        // create original data
        let mut oracle = create_test_oracle();
        let key = PreimageKey::new([42u8; 32], PreimageKeyType::Sha256);
        let value = b"test_persistent_data".to_vec();
        oracle.insert_preimage(key, value.clone());

        // write to disk
        oracle.write_to_disk(&file_path)?;
        assert!(file_path.exists());

        // create new oracle and read
        let mut new_oracle = MockOracle {
            map: HashMap::new(),
        };
        new_oracle.read_from_disk(&file_path)?;

        // verify data
        assert_eq!(new_oracle.preimage_count(), oracle.preimage_count());
        assert_eq!(new_oracle.map.get(&key).unwrap(), &value);

        Ok(())
    }

    #[test]
    fn test_different_key_types() -> std::io::Result<()> {
        let temp_dir = tempdir()?;
        let file_path = temp_dir.path().join("keys_test.db");

        let mut oracle = MockOracle {
            map: HashMap::new(),
        };

        // test different key types
        let local_key = PreimageKey::new_local(2001);
        let sha256_key = PreimageKey::new([1u8; 32], PreimageKeyType::Sha256);
        let keccak256_key = PreimageKey::new([2u8; 32], PreimageKeyType::Keccak256);

        let value1 = b"local_value".to_vec();
        let value2 = b"sha256_value".to_vec();
        let value3 = b"keccak256_value".to_vec();

        oracle.insert_preimage(local_key, value1.clone());
        oracle.insert_preimage(sha256_key, value2.clone());
        oracle.insert_preimage(keccak256_key, value3.clone());

        // 保存和加载
        oracle.write_to_disk(&file_path)?;

        let mut new_oracle = MockOracle {
            map: HashMap::new(),
        };
        new_oracle.read_from_disk(&file_path)?;

        // 验证所有类型的 key 都正确保存和加载
        assert_eq!(new_oracle.map.get(&local_key).unwrap(), &value1);
        assert_eq!(new_oracle.map.get(&sha256_key).unwrap(), &value2);
        assert_eq!(new_oracle.map.get(&keccak256_key).unwrap(), &value3);

        Ok(())
    }

    #[test]
    fn test_clear_functionality() {
        let mut oracle = create_test_oracle();

        // 添加一些数据
        let key = PreimageKey::new_local(3001);
        let value = b"test_value".to_vec();
        oracle.insert_preimage(key, value);

        let initial_count = oracle.preimage_count();
        assert!(initial_count > 0);

        // 清空数据
        oracle.clear();
        assert_eq!(oracle.preimage_count(), 0);

        let stats = oracle.storage_stats();
        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.total_value_size, 0);
    }

    #[test]
    fn test_invalid_file_format() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("invalid.db");

        // 写入无效的文件内容
        std::fs::write(&file_path, b"invalid_content").unwrap();

        let mut oracle = MockOracle {
            map: HashMap::new(),
        };

        // 尝试读取应该失败
        let result = oracle.read_from_disk(&file_path);
        assert!(result.is_err());
        assert_eq!(oracle.preimage_count(), 0);
    }

    #[test]
    fn test_storage_stats() {
        let mut oracle = create_test_oracle();

        // 获取初始统计信息
        let initial_stats = oracle.storage_stats();
        assert_eq!(initial_stats.entry_count, 6); // boot info 条目

        // 添加一些数据
        let key1 = PreimageKey::new_local(4001);
        let key2 = PreimageKey::new_local(4002);
        let value1 = vec![1u8; 100]; // 100 bytes
        let value2 = vec![2u8; 200]; // 200 bytes

        oracle.insert_preimage(key1, value1);
        oracle.insert_preimage(key2, value2);

        let final_stats = oracle.storage_stats();
        assert_eq!(final_stats.entry_count, initial_stats.entry_count + 2);
        assert!(final_stats.total_value_size >= initial_stats.total_value_size + 300);
        assert!(final_stats.estimated_memory_usage > final_stats.total_value_size);
    }

    #[tokio::test]
    async fn test_async_interface() -> anyhow::Result<()> {
        let mut oracle = create_test_oracle();

        // 插入测试数据
        let key = PreimageKey::new_local(5001);
        let value = b"async_test_value".to_vec();
        oracle.insert_preimage(key, value.clone());

        // 测试 async get
        let retrieved = oracle.get(key).await?;
        assert_eq!(retrieved, value);

        // 测试 async get_exact
        let mut buf = vec![0u8; value.len()];
        oracle.get_exact(key, &mut buf).await?;
        assert_eq!(buf, value);

        // 测试 HintWriter 接口
        oracle.write("test hint").await?;

        Ok(())
    }
}
