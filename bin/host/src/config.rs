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

use crate::args::KailuaHostArgs;
use alloy::providers::{Provider, ProviderBuilder, RootProvider};
use anyhow::Context;
use kailua_client::provider::OpNodeProvider;
use soon_primitives::rollup_config::SoonRollupConfig;
use opentelemetry::global::tracer;
use opentelemetry::trace::{FutureExt, TraceContextExt, Tracer};
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::fs;
use tracing::{debug, info};

pub async fn generate_rollup_config(
    cfg: &mut KailuaHostArgs,
    tmp_dir: &TempDir,
) -> anyhow::Result<SoonRollupConfig> {
    // generate a RollupConfig for the target network
    Ok(match cfg.kona.read_rollup_config().ok() {
        Some(rollup_config) => rollup_config,
        None => {
            let tmp_cfg_file = tmp_dir.path().join("rollup-config.json");
            info!("Fetching rollup config from nodes.");
            fetch_rollup_config(
                cfg.op_node_address.as_ref().unwrap().as_str(),
                cfg.kona
                    .l2_node_address
                    .clone()
                    .expect("Missing l2-node-address")
                    .as_str(),
                Some(&tmp_cfg_file),
            )
                .await?;
            cfg.kona.rollup_config_path = Some(tmp_cfg_file);
            debug!("{:?}", cfg.kona.rollup_config_path);
            cfg.kona.read_rollup_config()?
        }
    })
}

pub async fn fetch_rollup_config(
    op_node_address: &str,
    l2_node_address: &str,
    json_file_path: Option<&PathBuf>,
) -> anyhow::Result<SoonRollupConfig> {
    let tracer = tracer("kailua");
    let context = opentelemetry::Context::current_with_span(tracer.start("fetch_rollup_config"));

    let op_node_provider = OpNodeProvider(RootProvider::new_http(op_node_address.try_into()?));
    let l2_node_provider = ProviderBuilder::new().connect_http(l2_node_address.try_into()?);

    let mut rollup_config: Value = op_node_provider
        .rollup_config()
        .with_context(context.clone())
        .await
        .context("rollup_config")?;

    debug!("Rollup config: {:?}", rollup_config);

    let chain_config: Value =
        l2_node_provider
            .client()
            .request_noparams("debug_chainConfig")
            .with_context(context.with_span(
                tracer.start_with_context("ReqwestProvider::debug_chainConfig", &context),
            ))
            .await
            .context("debug_chainConfig")?;

    debug!("ChainConfig: {:?}", chain_config);

    // fork times
    for fork in &[
        "regolithTime",
        "canyonTime",
        "deltaTime",
        "ecotoneTime",
        "fjordTime",
        "graniteTime",
        "holoceneTime",
    ] {
        if let Some(value) = chain_config[fork].as_str() {
            rollup_config[fork] = json!(value);
        }
    }

    // export
    let ser_config = serde_json::to_string(&rollup_config)?;
    if let Some(json_file_path) = json_file_path {
        fs::write(json_file_path, &ser_config).await?;
    }

    Ok(serde_json::from_str(&ser_config)?)
}
