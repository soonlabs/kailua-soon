// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use alloy_consensus::TxEip4844Variant::{TxEip4844, TxEip4844WithSidecar};
use alloy_consensus::{Header, Receipt, ReceiptEnvelope, TxEnvelope};
use alloy_eips::{BlockNumberOrTag, Decodable2718};
use alloy_primitives::map::B256Map;
use alloy_primitives::{Sealed, B256};
use alloy_rlp::Decodable;
use async_trait::async_trait;
use kona_mpt::{OrderedListWalker, TrieNode, TrieProvider};
use kona_preimage::{CommsClient, PreimageKey, PreimageKeyType};
use kona_proof::errors::OracleProviderError;
use kona_proof::HintType;
use soon_derive::traits::ChainProvider;
use soon_primitives::blocks::{BlockInfo, L1Header, L1Transaction};
use std::sync::Arc;
use std::sync::RwLock;
use alloy_consensus::transaction::SignerRecoverable;

/// The oracle-backed L1 chain provider for the client program.
/// Forked from [kona_proof::l1::OracleL1ChainProvider]
#[derive(Debug, Clone)]
pub struct OracleL1ChainProvider<T: CommsClient> {
    /// The preimage oracle client.
    pub oracle: Arc<T>,
    /// The chain of block headers traversed
    pub headers: Arc<RwLock<Vec<Sealed<L1Header>>>>,
    /// The index of each
    pub headers_map: Arc<RwLock<B256Map<usize>>>,
}

impl<T: CommsClient> OracleL1ChainProvider<T> {
    /// Creates a new [OracleL1ChainProvider] with the given boot information and oracle client.
    pub async fn new(l1_head: B256, oracle: Arc<T>) -> Result<Self, OracleProviderError> {
        let (headers, headers_map) = if l1_head.is_zero() {
            Default::default()
        } else {
            // Fetch the header RLP from the oracle.
            HintType::L1BlockHeader
                .with_data(&[l1_head.as_ref()])
                .send(oracle.as_ref())
                .await?;
            let header_rlp = oracle.get(PreimageKey::new_keccak256(*l1_head)).await?;

            // Decode the header RLP into a Header.
            let l1_header: L1Header = Header::decode(&mut header_rlp.as_slice())
                .map_err(OracleProviderError::Rlp)?
                .into();

            (
                vec![l1_header.seal(l1_head)],
                B256Map::from_iter(vec![(l1_head, 0usize)]),
            )
        };

        Ok(Self {
            oracle,
            headers: Arc::new(RwLock::new(headers)),
            headers_map: Arc::new(RwLock::new(headers_map)),
        })
    }
}

#[async_trait]
impl<T: CommsClient + Sync + Send> ChainProvider for OracleL1ChainProvider<T> {
    type Error = OracleProviderError;

    /// Retrieves and returns a block header by its hash.
    ///
    /// This function attempts to retrieve a block header by its hash (`hash`),
    /// prioritizing locally cached headers to minimize the need for external requests.
    /// If the header is not found in the cache, it fetches the data using the
    /// connected oracle.
    ///
    /// # Parameters
    /// - `hash`: The hash (`[u8; 32]` format, wrapped in `B256`) identifying the block header.
    ///
    /// # Returns
    /// - `Ok(Header)`: The successfully retrieved and decoded block header.
    /// - `Err(Self::Error)`: An error that occurred during the retrieval or decoding process.
    ///
    /// # Process
    /// 1. Check if the header is cached in `self.headers_map`. If found, it is fetched
    ///    from local storage, unsealed, and returned.
    /// 2. If not cached, the function sends a request (using a `HintType`) for the
    ///    header data via the oracle.
    /// 3. Retrieves the header's RLP data from the oracle using `PreimageKey::new_keccak256`.
    /// 4. Decodes the RLP-encoded header into a `Header` structure.
    /// 5. Returns the decoded `Header` or an error if decoding fails.
    ///
    /// # Errors
    /// - Returns a `Self::Error` if the oracle request, response retrieval, or
    ///   RLP decoding fails.
    async fn header_by_hash(&self, hash: B256) -> Result<L1Header, Self::Error> {
        // Use cached headers
        {
            let headers_map = self.headers_map.read().unwrap();
            let headers = self.headers.read().unwrap();
            if let Some(index) = headers_map.get(&hash) {
                return Ok(headers[*index].clone().unseal().into());
            }
        }

        // Fetch the header RLP from the oracle.
        HintType::L1BlockHeader
            .with_data(&[hash.as_ref()])
            .send(self.oracle.as_ref())
            .await?;
        let header_rlp = self.oracle.get(PreimageKey::new_keccak256(*hash)).await?;

        // Decode the header RLP into a Header.
        let header =
            Header::decode(&mut header_rlp.as_slice()).map_err(OracleProviderError::Rlp)?;
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

    /// Retrieves block information for a specific block number asynchronously.
    ///
    /// This function attempts to retrieve information about a block specified by its number. It works
    /// by navigating the blockchain headers stored in memory, accessing the required block's details,
    /// and constructing a `BlockInfo` structure with relevant data such as hash, number, parent hash,
    /// and timestamp.
    ///
    /// # Arguments
    /// * `block_number` - A `u64` representing the block number whose information is being retrieved.
    ///
    /// # Returns
    /// A `Result` which:
    /// - On success, contains a `BlockInfo` struct with the requested block's details.
    /// - On failure, contains an error of type `Self::Error`, such as `OracleProviderError`.
    ///
    /// # Errors
    /// - Returns `OracleProviderError::BlockNumberPastHead` if the requested `block_number` is greater
    ///   than the number of the current "head" block.
    /// - Returns other errors propagated from asynchronous operations such as fetching a header based
    ///   on its hash.
    ///
    /// # Behavior
    /// 1. First, checks if the block number is greater than the head block's number. If true,
    ///    returns an error.
    /// 2. Calculates the index of the requested block in the local header cache.
    /// 3. Iteratively walks back through cached blockchain headers if the desired block is not yet
    ///    cached, fetching additional parent headers as needed via `header_by_hash`.
    /// 4. Constructs and returns a `BlockInfo` struct containing the required block's hash, number,
    ///    parent hash, and timestamp.
    async fn block_info_by_number(
        &self,
        block_number: BlockNumberOrTag,
    ) -> Result<BlockInfo, Self::Error> {
        let block_number = block_number.as_number().unwrap_or_default();

        // Check if the block number is in range. If not, we can fail early.
        {
            let headers = self.headers.read().unwrap();
            if block_number > headers[0].number {
                return Err(OracleProviderError::BlockNumberPastHead(
                    block_number,
                    headers[0].number,
                ));
            }
        }

        // Calculate header index
        let header_index = {
            let headers = self.headers.read().unwrap();
            (headers[0].number - block_number) as usize
        };

        // Walk back the block headers to the desired block number.
        loop {
            let need_more_headers = {
                let headers_map = self.headers_map.read().unwrap();
                headers_map.len() <= header_index
            };

            if !need_more_headers {
                break;
            }

            // Get the parent hash of the last cached header
            let header_hash = {
                let headers = self.headers.read().unwrap();
                let headers_map = self.headers_map.read().unwrap();
                headers[headers_map.len() - 1].parent_hash
            };

            let header = self.header_by_hash(header_hash).await?;

            // Acquire write locks to modify both collections
            {
                let mut headers_map = self.headers_map.write().unwrap();
                let mut headers = self.headers.write().unwrap();
                headers_map.insert(header_hash, headers.len());
                headers.push(header.seal(header_hash));
            }
        }

        // Get the final header
        let headers = self.headers.read().unwrap();
        let header = &headers[header_index];

        Ok(BlockInfo {
            hash: header.hash(),
            number: header.number,
            parent_hash: header.parent_hash,
            timestamp: header.timestamp,
        })
    }

    async fn receipts_by_hash(&self, hash: B256) -> Result<Vec<Receipt>, Self::Error> {
        // Fetch the block header to find the receipts root.
        let header = self.header_by_hash(hash).await?;

        // Send a hint for the block's receipts, and walk through the receipts trie in the header to
        // verify them.
        HintType::L1Receipts
            .with_data(&[hash.as_ref()])
            .send(self.oracle.as_ref())
            .await?;
        let trie_walker = OrderedListWalker::try_new_hydrated(header.receipts_root, self)
            .map_err(OracleProviderError::TrieWalker)?;

        // Decode the receipts within the receipts trie.
        let receipts = trie_walker
            .into_iter()
            .map(|(_, rlp)| {
                let envelope = ReceiptEnvelope::decode_2718(&mut rlp.as_ref())?;
                Ok(envelope.as_receipt().expect("Infallible").clone())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(OracleProviderError::Rlp)?;

        Ok(receipts)
    }

    async fn get_block_transactions_by_hash(
        &self,
        hash: B256,
    ) -> Result<Vec<L1Transaction>, Self::Error> {
        // Fetch the block header to construct the block info.
        let header = self.header_by_hash(hash).await?;

        // Send a hint for the block's transactions, and walk through the transactions trie in the
        // header to verify them.
        HintType::L1Transactions
            .with_data(&[hash.as_ref()])
            .send(self.oracle.as_ref())
            .await?;
        let trie_walker = OrderedListWalker::try_new_hydrated(header.transactions_root, self)
            .map_err(OracleProviderError::TrieWalker)?;

        // Decode the transactions within the transactions trie.
        let transactions = trie_walker
            .into_iter()
            .map(|(_, rlp)| {
                // note: not short-handed for error type coersion w/ `?`.
                let rlp = TxEnvelope::decode_2718(&mut rlp.as_ref())?;
                Ok(rlp)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(OracleProviderError::Rlp)?;

        let l1_transactions = transactions
            .iter()
            .map(|tx| {
                let (to, data) = match tx {
                    TxEnvelope::Legacy(tx) => (tx.tx().to.into_to(), &tx.tx().input),
                    TxEnvelope::Eip2930(tx) => (tx.tx().to.into_to(), &tx.tx().input),
                    TxEnvelope::Eip1559(tx) => (tx.tx().to.into_to(), &tx.tx().input),
                    TxEnvelope::Eip4844(tx) => match tx.tx() {
                        TxEip4844(tx) => (Some(tx.to), &tx.input),
                        TxEip4844WithSidecar(tx) => (Some(tx.tx().to), &tx.tx().input),
                    },
                    TxEnvelope::Eip7702(tx) => (Some(tx.tx().to), &tx.tx().input),
                };
                Ok(L1Transaction {
                    hash: *tx.hash(),
                    from: tx.recover_signer_unchecked().unwrap(),
                    to,
                    input: data.to_vec(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(OracleProviderError::Rlp)?;

        Ok(l1_transactions)
    }
}

impl<T: CommsClient> TrieProvider for OracleL1ChainProvider<T> {
    type Error = OracleProviderError;

    fn trie_node_by_hash(&self, key: B256) -> Result<TrieNode, Self::Error> {
        // On L1, trie node preimages are stored as keccak preimage types in the oracle. We assume
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

    fn bank_hash(&self, _block_number: u64) -> Result<B256, Self::Error> {
        unimplemented!("L2 bank hash is not supported for L1 chain provider")
    }

    fn block_time(&self, _block_number: u64) -> Result<i64, Self::Error> {
        unimplemented!("L2 block time is not supported for L1 chain provider")
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::oracle::vec::VecOracle;

//     #[tokio::test]
//     async fn test_concurrent_access() {
//         let oracle = Arc::new(VecOracle::new());
//         let provider = Arc::new(
//             OracleL1ChainProvider::new(B256::ZERO, oracle)
//                 .await
//                 .unwrap(),
//         );

//         // Test concurrent read access
//         let handles: Vec<_> = (0..10)
//             .map(|_| {
//                 let provider = Arc::clone(&provider);
//                 tokio::spawn(async move {
//                     let headers = provider.headers.read().await;
//                     let headers_map = provider.headers_map.read().await;
//                     assert_eq!(headers.len(), 1);
//                     assert_eq!(headers_map.len(), 1);
//                 })
//             })
//             .collect();

//         for handle in handles {
//             handle.await.unwrap();
//         }
//     }
// }
