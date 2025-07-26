use alloy_consensus::{Header, Receipt};
use alloy_eips::{BlockNumberOrTag, BlockNumHash};
use alloy_primitives::{Address, B256, Bytes, keccak256, U160, U256};
use async_trait::async_trait;
use kona_driver::PipelineCursor;
use kona_executor::TrieDBProvider;
use kona_mpt::{TrieHinter, TrieNode, TrieProvider};
use kona_preimage::{CommsClient, PreimageKey, PreimageKeyType};
use kona_proof::errors::OracleProviderError;
use kona_proof::l2::CursorSetter;
use soon_derive::traits::{ChainProvider, L2ChainProvider};
use soon_primitives::blocks::{BlockInfo, L1Header, L1Transaction, L2BlockInfo, str_block_hash_to};
use soon_primitives::l2blocks::L2Block;
use alloy_eips::NumHash;
use soon_primitives::system::SystemConfig;
use std::fmt::Debug;
use std::sync::Arc;
use alloy_rlp::Decodable;
use l1_block_info::instruction::L1BlockInfoInstruction;
use spin::RwLock;

/// Test L1 Chain Provider for testing purposes
#[derive(Debug, Clone)]
pub struct TestOracleL1ChainProvider<O> {
    oracle: Arc<O>,
}

impl<O> TestOracleL1ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    pub fn new(oracle: Arc<O>) -> Self {
        Self { oracle }
    }
}

#[async_trait]
impl<O> ChainProvider for TestOracleL1ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    type Error = OracleProviderError;

    async fn header_by_hash(&self, hash: B256) -> Result<L1Header, Self::Error> {
        let header_bytes = self.oracle
            .get(PreimageKey::new(*hash, PreimageKeyType::Keccak256))
            .await?;

        // Decode the header RLP into a Header.
        let header =
            Header::decode(&mut header_bytes.as_slice()).map_err(OracleProviderError::Rlp)?;
        Ok(header.into())
    }

    async fn block_info_by_hash(&self, hash: B256) -> Result<BlockInfo, Self::Error> {
        let header = self.header_by_hash(hash).await?;
        Ok(BlockInfo {
            hash: header.hash,
            number: header.number,
            parent_hash: header.parent_hash,
            timestamp: header.timestamp,
        })
    }

    async fn block_info_by_number(
        &self,
        block_number: BlockNumberOrTag,
    ) -> Result<BlockInfo, Self::Error> {
        let number_bytes = block_number.as_number().unwrap().to_be_bytes();
        let number_hash = keccak256(number_bytes.as_ref());
        let header_bytes = self.oracle.get(PreimageKey::new_keccak256(*number_hash)).await?;

        let header: L1Header = Header::decode(&mut header_bytes.as_slice()).map_err(OracleProviderError::Rlp)?.into();
        Ok(BlockInfo {
            hash: header.hash,
            number: header.number,
            parent_hash: header.parent_hash,
            timestamp: header.timestamp,
        })
    }

    async fn receipts_by_hash(&self, _hash: B256) -> Result<Vec<Receipt>, Self::Error> {
        // Return empty receipts for testing
        Ok(vec![])
    }

    async fn get_block_transactions_by_hash(&self, _hash: B256) -> Result<Vec<L1Transaction>, Self::Error> {
        // Return empty transactions for testing
        Ok(vec![])
    }
}

/// Test L2 Chain Provider for testing purposes
#[derive(Debug, Clone)]
pub struct TestOracleL2ChainProvider<O> {
    oracle: Arc<O>,
    cursor: Option<Arc<RwLock<PipelineCursor>>>,
}

impl<O> TestOracleL2ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    pub fn new(oracle: Arc<O>) -> Self {
        Self {
            oracle,
            cursor: None,
        }
    }

    fn get_l1_block_info(
        &self,
        block: L2Block,
    ) -> Result<L1BlockInfoInstruction, OracleProviderError> {
        let l1_block_info_tx = block.transactions.first().ok_or(
            OracleProviderError::FetchBlockInfoFailed("No l1 block info tx found".to_string()),
        )?;
        let l1_block_info_tx_data =
            l1_block_info_tx.0.message.instructions().first().ok_or(
                OracleProviderError::FetchBlockInfoFailed("No instruction found".to_string()),
            )?;
        let l1_block_info_instruction =
            L1BlockInfoInstruction::unpack(l1_block_info_tx_data.data.as_slice())
                .map_err(|err| OracleProviderError::FetchBlockInfoFailed(err.to_string()))?;
        Ok(l1_block_info_instruction)
    }
}

#[async_trait]
impl<O> L2ChainProvider for TestOracleL2ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    type Error = OracleProviderError;

    async fn l2_block_info_by_number(&mut self, number: u64) -> Result<L2BlockInfo, Self::Error> {
        let block = self.block_by_number(number).await?;
        let block_info = BlockInfo::new(
            str_block_hash_to(block.blockhash.as_str()),
            number,
            str_block_hash_to(block.previous_blockhash.as_str()),
            block.block_time,
        );
        let l1_block_info_instruction = self.get_l1_block_info(block)?;
        match l1_block_info_instruction {
            L1BlockInfoInstruction::UpdateL1BlockInfo {
                number,
                timestamp: _,
                base_fee: _,
                hash,
                sequence_number,
                batcher_hash: _,
                fee_overhead: _,
                fee_scalar: _,
                gas: _,
                is_system_tx: _,
            } => Ok(L2BlockInfo {
                block_info,
                l1_origin: BlockNumHash { number, hash: B256::from(hash) },
                seq_num: sequence_number,
            }),
            _ => Err(OracleProviderError::FetchBlockInfoFailed(
                "Invalid l1 block info instruction".to_string(),
            )),
        }
    }

    async fn block_by_number(&mut self, number: u64) -> Result<L2Block, Self::Error> {
        let number_bytes = number.to_be_bytes();
        let number_hash = keccak256(number_bytes.as_ref());
        let block_bytes = self.oracle.get(PreimageKey::new_keccak256(*number_hash)).await?;

        Decodable::decode(&mut block_bytes.as_slice()).map_err(OracleProviderError::Rlp)
    }

    async fn system_config_by_number(&mut self, number: u64) -> Result<SystemConfig, Self::Error> {
        let block = self.block_by_number(number).await?;
        let l1_block_info_instruction = self.get_l1_block_info(block)?;
        match l1_block_info_instruction {
            L1BlockInfoInstruction::UpdateL1BlockInfo {
                number: _,
                timestamp: _,
                base_fee: _,
                hash: _,
                sequence_number: _,
                batcher_hash,
                fee_overhead: _,
                fee_scalar: _,
                gas: _,
                is_system_tx: _,
            } => {
                let address_value = U160::from_le_slice(&batcher_hash[..20]);
                Ok(SystemConfig {
                    batcher_address: Address::from(address_value),
                    ..Default::default()
                })
            }
            _ => Err(OracleProviderError::FetchBlockInfoFailed(
                "Invalid l1 block info instruction".to_string(),
            )),
        }
    }
}

impl<O> TrieProvider for TestOracleL2ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    type Error = OracleProviderError;

    fn trie_node_by_hash(&self, key: B256) -> Result<TrieNode, Self::Error> {
        // On L2, trie node preimages are stored as keccak preimage types in the oracle. We assume
        // that a hint for these preimages has already been sent, prior to this call.
        kona_proof::block_on(async move {
            TrieNode::decode(
                &mut self
                    .oracle
                    .get(PreimageKey::new(*key, PreimageKeyType::Keccak256))
                    .await
                    .map_err(OracleProviderError::Preimage)?
                    .as_ref(),
            )
                .map_err(OracleProviderError::Rlp)
        })
    }
}

impl<O> TrieDBProvider for TestOracleL2ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    fn bytecode_by_hash(&self, hash: B256) -> Result<Bytes, Self::Error> {
        kona_proof::block_on(async move {
            self.oracle
                .get(PreimageKey::new(*hash, PreimageKeyType::Keccak256))
                .await
                .map(Bytes::from)
                .map_err(OracleProviderError::Preimage)
        })
    }
}

impl<O> TrieHinter for TestOracleL2ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    type Error = OracleProviderError;

    fn hint_trie_node(&self, _key: B256) -> Result<(), Self::Error> {
        // No-op for testing
        Ok(())
    }

    fn hint_account_proof(&self, _address: Address, _block_number: u64) -> Result<(), Self::Error> {
        // No-op for testing
        Ok(())
    }

    fn hint_storage_proof(&self, _address: Address, _index: U256, _block_number: u64) -> Result<(), Self::Error> {
        // No-op for testing
        Ok(())
    }
}

impl<O> CursorSetter for TestOracleL2ChainProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    fn set_cursor(&mut self, cursor: Arc<RwLock<PipelineCursor>>) {
        self.cursor = Some(cursor);
    }
}

/// Test DA Provider for testing purposes
#[derive(Debug, Clone)]
pub struct TestDaProvider<O> {
    oracle: Arc<O>,
}

impl<O> TestDaProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    pub fn new(oracle: Arc<O>) -> Self {
        Self { oracle }
    }
}

#[async_trait]
impl<O> soon_derive::traits::DAProvider for TestDaProvider<O>
where
    O: CommsClient + Send + Sync + Debug,
{
    type Error = OracleProviderError;

    async fn set_input(&self, _data: Vec<u8>) -> Result<Vec<u8>, Self::Error> {
        // Return empty data for testing
        Ok(vec![])
    }

    async fn get_input(&self, _data: Vec<u8>) -> Result<Vec<u8>, Self::Error> {
        // Return empty data for testing
        Ok(vec![])
    }
} 