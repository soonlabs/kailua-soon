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

pub mod core;
#[cfg(feature = "__test")]
mod soon_test;
pub mod stateless;
pub mod stitching;

/// Logs a given message under different logging mechanisms based on the target operating system.
///
/// # Parameters
/// - `msg`: A string slice representing the message to be logged.
///
/// # Platform-specific Behavior
/// - On a `zkvm` target operating system:
///   - Logs the message using the RISC Zero zkVM environment's logging mechanism (`risc0_zkvm::guest::env::log`).
/// - On other target operating systems:
///   - Logs the message using the `tracing` crate's `info!` macro.
pub fn log(msg: &str) {
    #[cfg(target_os = "zkvm")]
    risc0_zkvm::guest::env::log(msg);
    #[cfg(not(target_os = "zkvm"))]
    tracing::info!("{msg}");
}

#[cfg(all(test, feature = "__test"))]
#[cfg_attr(coverage_nightly, coverage(off))]
pub mod tests {
    use crate::test::TestOracle;
    use kona_proof::BootInfo;

    #[test]
    fn test_oracle_cloning() {
        let oracle = TestOracle::new(BootInfo {
            l1_head: Default::default(),
            agreed_l2_output_root: Default::default(),
            claimed_l2_output_root: Default::default(),
            claimed_l2_block_number: 0,
            chain_id: 0,
            rollup_config: Default::default(),
        });
        let cloned = oracle.clone();
        // avoid double dropping
        assert!(cloned.temp_dir.is_none());
    }
}
