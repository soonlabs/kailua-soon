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

use crate::provider::beacon::BlobProvider;
use crate::{await_tel, retry_res_ctx};
use alloy::providers::RootProvider;
use opentelemetry::trace::{FutureExt, TraceContextExt, Tracer};
use soon_l2_chain_provider::chain_provider::L2BlockFetcher;

pub mod beacon;
pub mod optimism;

#[derive(clap::Args, Debug, Clone, Default)]
pub struct ProviderArgs {
    /// Address of the SOON-NODE endpoint to use.
    #[clap(long, env)]
    pub soon_node_url: String,
    /// Number of L2 outputs to fetch at once
    #[clap(long, env, default_value_t = 64)]
    pub op_rpc_concurrency: u64,
    /// Number of L2 blocks to delay observation by
    #[clap(long, env, default_value_t = 0)]
    pub soon_rpc_delay: u64,
    /// Address of the ethereum rpc endpoint to use (eth namespace required)
    #[clap(long, env)]
    pub eth_rpc_url: String,
    /// Address of the L1 Beacon API endpoint to use.
    #[clap(long, env)]
    pub beacon_rpc_url: String,
    /// Address of the DA proxy to use.
    #[clap(long, env)]
    pub da_proxy_url: Option<String>,
    /// Time (in seconds) between successive RPC polls
    #[clap(long, env, default_value_t = 6)]
    pub rpc_poll_interval: u64,

    #[clap(flatten)]
    pub timeouts: ProviderTimeoutArgs,
}

#[derive(clap::Args, Debug, Clone, Copy)]
pub struct ProviderTimeoutArgs {
    /// Timeout (seconds) for an SOON-NODE RPC request.
    #[clap(long, env, default_value_t = 5)]
    pub soon_node_timeout: u64,
    /// Timeout (seconds) for an ETH RPC request.
    #[clap(long, env, default_value_t = 2)]
    pub eth_rpc_timeout: u64,
    /// Timeout (seconds) for a BEACON RPC request.
    #[clap(long, env, default_value_t = 20)]
    pub beacon_rpc_timeout: u64,
}

impl Default for ProviderTimeoutArgs {
    fn default() -> Self {
        Self {
            soon_node_timeout: 5,
            eth_rpc_timeout: 2,
            beacon_rpc_timeout: 20,
        }
    }
}

impl ProviderTimeoutArgs {
    pub fn max(&self) -> u64 {
        self.soon_node_timeout
            .max(self.eth_rpc_timeout)
            .max(self.beacon_rpc_timeout)
    }
}

/// A collection of RPC providers for L1 and L2 data
pub struct SyncProvider {
    /// blobs provider
    pub da_provider: BlobProvider,
    /// Provider for L1 chain data
    pub l1_provider: RootProvider,
    /// Provider for L2 chain data
    pub l2_provider: L2BlockFetcher,
    /// Provider for DA proxy data
    pub da_proxy_provider: Option<RootProvider>,
}

impl SyncProvider {
    pub async fn new(args: &ProviderArgs) -> anyhow::Result<Self> {
        let tracer = opentelemetry::global::tracer("kailua");
        let context = opentelemetry::Context::current_with_span(tracer.start("SyncProvider::new"));

        let da_provider = await_tel!(
            context,
            tracer,
            "BlobProvider::new",
            retry_res_ctx!(BlobProvider::new(
                args.beacon_rpc_url.clone(),
                args.timeouts.beacon_rpc_timeout
            ))
        );

        let l1_provider = RootProvider::new_http(args.eth_rpc_url.as_str().try_into()?);

        let l2_provider = L2BlockFetcher::new_with_url(args.soon_node_url.as_str());

        let da_proxy_provider = if let Some(da_proxy_url) = args.da_proxy_url.clone() {
            Some(RootProvider::new_http(da_proxy_url.as_str().try_into()?))
        } else {
            None
        };

        Ok(Self {
            da_provider,
            l1_provider,
            l2_provider,
            da_proxy_provider,
        })
    }
}
