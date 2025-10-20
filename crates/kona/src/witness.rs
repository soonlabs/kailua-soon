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

use crate::blobs::BlobWitnessData;
use crate::boot::StitchedBootInfo;
use crate::driver::CachedDriver;
use crate::executor::Execution;
use crate::oracle::vec::VecOracle;
use crate::oracle::WitnessOracle;
use crate::precondition::Precondition;
use crate::rkyv::primitives::{AddressDef, B256Def};
use alloy_primitives::{Address, B256};
use std::fmt::Debug;

/// Represents the complete structure of a `Witness`, which is used to hold
/// the necessary data for authenticating a rollup state transition in the FPVM.
#[derive(Clone, Debug, Default, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Witness<O: WitnessOracle> {
    /// The witness oracle for preimage data preloaded in memory.
    pub oracle_witness: O,
    /// The witness oracle for preimage data streamed in on demand.
    pub stream_witness: O,
    /// Represents the witness data for blobs.
    pub blobs_witness: BlobWitnessData,
    /// Represents the address of the proof's payout recipient.
    #[rkyv(with = AddressDef)]
    pub payout_recipient_address: Address,
    /// Represents a hash value used for loading precondition validation data.
    #[rkyv(with = B256Def)]
    pub precondition_validation_data_hash: B256,
    /// A collection of stitched executions represented as a two-dimensional vector.
    ///
    /// # Structure:
    /// - The outer `Vec` represents a collection of execution groups.
    /// - Each inner `Vec<Execution>` contains a continuous series of `Execution` objects that
    ///   represent individual executions within a specific stitched group.
    ///
    /// # Notes:
    /// - Ensure all `Execution` objects within the groups are properly sorted.
    pub stitched_executions: Vec<Vec<Execution>>,
    /// An initial state for the derivation pipeline
    pub derivation_cache: Option<CachedDriver>,
    /// Whether to record a derivation trace precondition in the output journal
    pub trace_derivation: bool,
    /// A list of `StitchedBootInfo` instances to be stitched together from other proofs.
    pub stitched_preconditions: Vec<Precondition>,
    /// A list of `StitchedBootInfo` instances to be stitched together from other proofs.
    pub stitched_boot_info: Vec<StitchedBootInfo>,
    /// Represents the fault-proof virtual machine program image id.
    #[rkyv(with = B256Def)]
    pub fpvm_image_id: B256,
}

impl Witness<VecOracle> {
    /// Creates a deep copy of the current instance.
    ///
    /// This method performs a "deep clone" of the object by cloning all its fields,
    /// including any nested fields that implement the `deep_clone` method.
    /// This ensures that all references and internal data are duplicated,
    /// rather than pointing to the same objects.
    ///
    /// # Returns
    /// A new instance of the structure with all fields deeply cloned.
    pub fn deep_clone(&self) -> Self {
        let mut cloned_with_arc = self.clone();
        cloned_with_arc.oracle_witness = cloned_with_arc.oracle_witness.deep_clone();
        cloned_with_arc.stream_witness = cloned_with_arc.stream_witness.deep_clone();
        cloned_with_arc
    }
}
