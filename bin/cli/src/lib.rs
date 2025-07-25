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

use kailua_client::telemetry::TelemetryArgs;
use std::path::PathBuf;

pub mod bench;
pub mod channel;
pub mod config;
pub mod fast_track;
pub mod fault;
pub mod propose;
pub mod retry;
pub mod stall;
pub mod sync;
pub mod transact;
pub mod validate;

pub const KAILUA_GAME_TYPE: u32 = 1337;

#[derive(clap::Parser, Debug, Clone)]
#[command(name = "kailua-cli")]
#[command(bin_name = "kailua-cli")]
#[command(author, version, about, long_about = None)]
#[allow(clippy::large_enum_variant)]
pub enum KailuaCli {
    Config(config::ConfigArgs),
    FastTrack(fast_track::FastTrackArgs),
    Propose(propose::ProposeArgs),
    Validate(validate::ValidateArgs),
    TestFault(fault::FaultArgs),
    Benchmark(bench::BenchArgs),
}

#[derive(clap::Args, Debug, Clone)]
pub struct CoreArgs {
    #[arg(long, short, help = "Verbosity level (0-4)", action = clap::ArgAction::Count)]
    pub v: u8,

    /// Address of the OP-NODE endpoint to use
    #[clap(long, env)]
    pub op_node_url: String,
    /// Address of the OP-GETH endpoint to use (eth and debug namespace required).
    #[clap(long, env)]
    pub op_geth_url: String,
    /// Address of the ethereum rpc endpoint to use (eth namespace required)
    #[clap(long, env)]
    pub eth_rpc_url: String,
    /// Address of the L1 Beacon API endpoint to use.
    #[clap(long, env)]
    pub beacon_rpc_url: String,

    #[cfg(feature = "devnet")]
    #[clap(long, env, default_value_t = 0)]
    pub delay_l2_blocks: u64,
    #[cfg(feature = "devnet")]
    #[clap(long, env, default_value_t = 0)]
    pub delay_l1_heads: u64,

    /// Directory to use for caching data
    #[clap(long, env)]
    pub data_dir: Option<PathBuf>,
}

impl KailuaCli {
    pub fn verbosity(&self) -> u8 {
        match self {
            KailuaCli::Config(args) => args.v,
            KailuaCli::FastTrack(args) => args.v,
            KailuaCli::Propose(args) => args.core.v,
            KailuaCli::Validate(args) => args.core.v,
            KailuaCli::TestFault(args) => args.propose_args.core.v,
            KailuaCli::Benchmark(args) => args.core.v,
        }
    }

    pub fn data_dir(&self) -> Option<PathBuf> {
        match self {
            KailuaCli::Propose(args) => args.core.data_dir.clone(),
            KailuaCli::Validate(args) => args.core.data_dir.clone(),
            _ => None,
        }
    }

    pub fn otlp_endpoint(&self) -> Option<String> {
        match self {
            KailuaCli::Config(args) => args.telemetry.otlp_collector.clone(),
            KailuaCli::FastTrack(args) => args.telemetry.otlp_collector.clone(),
            KailuaCli::Propose(args) => args.telemetry.otlp_collector.clone(),
            KailuaCli::Validate(args) => args.telemetry.otlp_collector.clone(),
            KailuaCli::TestFault(args) => args.propose_args.telemetry.otlp_collector.clone(),
            KailuaCli::Benchmark(args) => args.telemetry.otlp_collector.clone(),
        }
    }

    pub fn telemetry_args(&self) -> &TelemetryArgs {
        match self {
            KailuaCli::Config(args) => &args.telemetry,
            KailuaCli::FastTrack(args) => &args.telemetry,
            KailuaCli::Propose(args) => &args.telemetry,
            KailuaCli::Validate(args) => &args.telemetry,
            KailuaCli::TestFault(args) => &args.propose_args.telemetry,
            KailuaCli::Benchmark(args) => &args.telemetry,
        }
    }
}
