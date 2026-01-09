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

use anyhow::Context;
use kailua_build::KAILUA_RECOVER_SIGNER_TEST_ELF;
use kailua_build::KAILUA_RECOVER_SIGNER_TEST_ID;
use alloy::primitives::{Address, U256};
use risc0_zkvm::{default_prover, ExecutorEnv, ProverOpts};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Serialize, Deserialize)]
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

pub fn run_recover_signer_test() -> anyhow::Result<()> {
    println!("Running recover_signer test in zkvm...");

    // Build Legacy transaction with provided data from the test case
    let nonce = 242u64;
    let gas_limit = 21000u64;
    let gas_price = 20900000000u128;
    let value = 32370000000000000u128;
    let chain_id = Some(11155111u64);
    let to = Address::from_str("0x359a68f67966247a34e07694493e0d00c99a1756")?;
    let input = Vec::<u8>::new();

    // Create signature from provided data
    // r = 111197453629367114907912549862485227720359187220219358471218136821626017544888
    // s = 16069675716490115033286433543232569847835186933082730357946014073768762936666
    // y_parity = false
    let r = U256::from_str_radix(
        "111197453629367114907912549862485227720359187220219358471218136821626017544888",
        10,
    )?;
    let s = U256::from_str_radix(
        "16069675716490115033286433543232569847835186933082730357946014073768762936666",
        10,
    )?;
    let y_parity = false;

    let tx_data = TransactionData {
        nonce,
        gas_limit,
        gas_price,
        value,
        chain_id,
        to_address: to,
        input,
        signature_r: r,
        signature_s: s,
        y_parity,
    };

    println!("Transaction data:");
    println!("  nonce: {}", tx_data.nonce);
    println!("  gas_limit: {}", tx_data.gas_limit);
    println!("  gas_price: {}", tx_data.gas_price);
    println!("  value: {}", tx_data.value);
    println!("  chain_id: {:?}", tx_data.chain_id);
    println!("  to: {:?}", tx_data.to_address);
    println!("  signature_r: {:?}", tx_data.signature_r);
    println!("  signature_s: {:?}", tx_data.signature_s);
    println!("  y_parity: {}", tx_data.y_parity);

    // Create executor environment
    let mut env_builder = ExecutorEnv::builder();
    env_builder.write(&tx_data)?;
    let env = env_builder.build()?;

    // Run the guest program
    println!("Executing guest program...");
    let prover = default_prover();
    let prover_opts = ProverOpts::succinct();
    let receipt = prover
        .prove_with_opts(env, KAILUA_RECOVER_SIGNER_TEST_ELF, &prover_opts)
        .map_err(|e| anyhow::anyhow!("Failed to prove: {:?}", e))?;

    // Verify the receipt
    println!("Verifying receipt...");
    receipt
        .receipt
        .verify(KAILUA_RECOVER_SIGNER_TEST_ID)
        .map_err(|e| anyhow::anyhow!("Failed to verify receipt: {:?}", e))?;

    println!("Receipt verified successfully!");

    // Read the result from the journal (true for success, false for failure)
    let journal = &receipt.receipt.journal;
    let success: bool = journal.decode().context("Failed to decode journal")?;

    if success {
        println!("✓ Successfully recovered signer address");
    } else {
        println!("✗ Failed to recover signer");
    }

    Ok(())
}
