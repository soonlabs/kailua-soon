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

use alloy_primitives::hex::FromHex;
use alloy_primitives::keccak256;
use kona_host::KeyValueStore;
use kona_preimage::PreimageKey;

#[allow(dead_code)]
fn dump_payload_to_kv_store(payload: &serde_json::Value, kv: &mut dyn KeyValueStore) -> u64 {
    if let Some(obj) = payload.as_object() {
        obj.iter()
            .map(|(k, v)| save_hex_preimage_to_kv(k, kv) + dump_payload_to_kv_store(v, kv))
            .sum()
    } else if let Some(seq) = payload.as_array() {
        seq.iter().map(|v| dump_payload_to_kv_store(v, kv)).sum()
    } else if let Some(v) = payload.as_str() {
        save_hex_preimage_to_kv(v, kv)
    } else {
        0
    }
}

#[allow(dead_code)]
fn save_hex_preimage_to_kv(preimage: &str, kv: &mut dyn KeyValueStore) -> u64 {
    alloy_primitives::Bytes::from_hex(preimage)
        .map(|preimage| {
            let computed_hash = keccak256(preimage.as_ref());
            let key = PreimageKey::new_keccak256(*computed_hash);
            let size = preimage.len() as u64;
            kv.set(key.into(), preimage.into()).unwrap();
            size
        })
        .unwrap_or(0)
}
