// Copyright 2024 RISC Zero, Inc.
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

use crate::await_tel;
use alloy::primitives::B256;
use anyhow::Context;
use opentelemetry::global::tracer;
use opentelemetry::trace::FutureExt;
use opentelemetry::trace::{TraceContextExt, Tracer};
use serde_json::Value;
use std::str::FromStr;
use jsonrpsee::http_client::{HttpClient};
use jsonrpsee::rpc_params;
use jsonrpsee::core::client::ClientT;
use soon_primitives::l2blocks::L2Block;

#[derive(Clone)]
pub struct SoonNodeProvider(pub HttpClient);

impl SoonNodeProvider {
    pub async fn get_block_by_number(&self, number: u64) -> anyhow::Result<L2Block> {
        Ok(L2Block::default())
    }
    pub async fn output_at_block(&self, output_block_number: u64) -> anyhow::Result<B256> {
        let tracer = tracer("kailua");
        let context = opentelemetry::Context::current_with_span(
            tracer.start("SoonNodeProvider::output_at_block"),
        );

        let params = rpc_params![output_block_number];
        let output_at_block: Value = await_tel!(
            context,
            tracer,
            "soon_outputAtBlock",
            self.0.request(
                "outputAtBlock",
                params
            )
        )
        .context(format!("soon_outputAtBlock {output_block_number}"))?;

        Ok(B256::from_str(
            output_at_block["outputRoot"].as_str().unwrap(),
        )?)
    }

    pub async fn sync_status(&self) -> anyhow::Result<Value> {
        let tracer = tracer("kailua");
        let context =
            opentelemetry::Context::current_with_span(tracer.start("SoonNodeProvider::sync_status"));

        Ok(await_tel!(
            context,
            tracer,
            "soon_syncStatus",
             self.0.request("getSyncStatus", rpc_params![])
        )?)
    }

    pub async fn rollup_config(&self) -> anyhow::Result<Value> {
        let tracer = tracer("kailua");
        let context = opentelemetry::Context::current_with_span(
            tracer.start("SoonNodeProvider::rollup_config"),
        );

        Ok(await_tel!(
            context,
            tracer,
            "soon_rollupConfig",
            self.0.request("getRollupConfig", rpc_params![])
        )?)
    }
}
