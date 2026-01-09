// Copyright 2024, 2025 RISC Zero, Inc.
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

// Import alloy types - these are now explicitly declared in Cargo.toml with exact versions
// to ensure HashMap hasher types are consistent with kailua-soon-kona
use alloy_consensus::transaction::{SignableTransaction, SignerRecoverable};
use alloy_consensus::{TxLegacy, TxEnvelope};
use alloy_primitives::{Address, Signature, U256};
use risc0_zkvm::guest::env;

fn main() {
    // Read transaction data from host
    let tx_data = env::read::<TransactionData>();
    
    risc0_zkvm::guest::env::log(&format!(
        "Testing recover_signer with: nonce={}, gas_limit={}, gas_price={}, value={}, chain_id={:?}",
        tx_data.nonce,
        tx_data.gas_limit,
        tx_data.gas_price,
        tx_data.value,
        tx_data.chain_id
    ));
    
    risc0_zkvm::guest::env::log(&format!(
        "Signature: r={:?}, s={:?}, y_parity={}",
        tx_data.signature_r,
        tx_data.signature_s,
        tx_data.y_parity
    ));
    
    // Build Legacy transaction with provided data
    let to = tx_data.to_address;
    let input = tx_data.input.clone();
    
    // Create signature from provided data
    let signature = Signature::new(tx_data.signature_r, tx_data.signature_s, tx_data.y_parity);
    
    // Create TxLegacy transaction
    let tx_legacy = TxLegacy {
        chain_id: tx_data.chain_id,
        nonce: tx_data.nonce,
        gas_price: tx_data.gas_price,
        gas_limit: tx_data.gas_limit,
        to: to.into(),
        value: U256::from(tx_data.value),
        input: input.into(),
    };
    
    // Sign the transaction
    let signed_tx = tx_legacy.into_signed(signature);
    
    // Create TxEnvelope::Legacy
    let tx_envelope = TxEnvelope::Legacy(signed_tx);
    
    risc0_zkvm::guest::env::log("Created TxEnvelope::Legacy");
    
    // Try to recover signer address
    let success = match tx_envelope.recover_signer() {
        Ok(recovered_address) => {
            risc0_zkvm::guest::env::log(&format!("Successfully recovered signer address: {:?}", recovered_address));
            risc0_zkvm::guest::env::log(&format!("Transaction hash: {:?}", tx_envelope.hash()));
            true
        }
        Err(e) => {
            let error_msg = format!(
                "Failed to recover signer from Legacy transaction. Error: {:?}, Hash: {:?}",
                e,
                tx_envelope.hash()
            );
            risc0_zkvm::guest::env::log(&error_msg);
            false
        }
    };
    
    // Commit the result (true for success, false for failure)
    env::commit(&success);
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct TransactionData {
    pub nonce: u64,
    pub gas_limit: u64,
    pub gas_price: u128,
    pub value: u128,
    pub chain_id: Option<u64>,
    pub to_address: Address,
    pub input: Vec<u8>,
    pub signature_r: U256,
    pub signature_s: U256,
    pub y_parity: bool,
}

