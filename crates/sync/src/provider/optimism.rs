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

use std::path::PathBuf;
use anyhow::Context;
use soon_primitives::rollup_config::SoonRollupConfig;
use opentelemetry::global::tracer;
use opentelemetry::trace::{FutureExt, TraceContextExt, Tracer};
use serde_json::{json, Value};
use std::str::FromStr;
use soon_l2_chain_provider::chain_provider::L2BlockFetcher;
use tokio::fs;
use tracing::log::warn;
use tracing::{debug, info};

pub async fn fetch_rollup_config(
    l2_node_address: &str,
    json_file_path: Option<&PathBuf>,
) -> anyhow::Result<SoonRollupConfig> {
    let tracer = tracer("kailua");
    let context = opentelemetry::Context::current_with_span(tracer.start("fetch_rollup_config"));

    let soon_node_provider = L2BlockFetcher::new_with_url(l2_node_address);

    let rollup_config: Value = soon_node_provider
        .rollup_config()
        .with_context(context.clone())
        .await
        .context("rollup_config")?;

    debug!("Rollup config: {:?}", rollup_config);

    // export
    let ser_config = serde_json::to_string(&rollup_config)?;
    if let Some(json_file_path) = json_file_path {
        fs::write(json_file_path, &ser_config).await?;
    }

    Ok(serde_json::from_str(&ser_config)?)
}
