# Setup

Make sure to first install the [prerequisites](quickstart.md#prerequisites) from the quickstart
section before proceeding.

## Installation

Before you can start migrating your rollup, you'll need to build and install Kailua's binaries by calling the following
commands from the root project directory:

```admonish tip
If you have modified the FPVM binary, you will need to build/install using `-F rebuild-fpvm`.
```

### CLI Binary
```shell
cargo install kailua-cli --path bin/cli --locked
```

### Prover Binary
```admonish info
At the cost of longer compilation time, you can embed the RISC Zero prover logic into `kailua-host` instead of having 
it utilize your locally installed RISC Zero `r0vm`.
To do this, add `-F prove` to the command below.
```

```admonish tip
For GPU-accelerated local proving, use one of the following feature flags:
* Apple: `-F metal`
* Nvidia: `-F cuda`
```

```shell
cargo install kailua-host --path bin/host --locked
```


## Configuration

Once your installation is successful, you should be able to run the following command to fetch the Kailua configuration
parameters for your rollup instance:

```shell
kailua-cli config --op-node-url [YOUR_OP_NODE_URL] --op-geth-url [YOUR_OP_GETH_URL] --eth-rpc-url [YOUR_ETH_RPC_URL]
```

Running the above command against the respective Base mainnet endpoints should produce the following output:
```
RISC0_VERSION: 2.1.0
FPVM_IMAGE_ID: 0x45CAF5B9B4BDB78DD80BB6551DF58117DB690AEB4FAD4D6904967BA0F3B5A62A
FPVM_ELF_SIZE: 31827560
CONTROL_ROOT: 0x539032186827B06719244873B17B2D4C122E2D02CFB1994FE958B2523B844576
CONTROL_ID: 0x04446E66D300EB7FB45C9726BB53C793DDA407A62E9601618BB43C5C14657AC0
RISC_ZERO_VERIFIER: 0x8EAB2D97DFCE405A1692A21B3FF3A172D593D319
GENESIS_TIMESTAMP: 1686789347
BLOCK_TIME: 2
ROLLUP_CONFIG_HASH: 0x0D2CFA12746085B0CBFCB344D7100F873BA380535218A0795F9F4DE43EA92AE7
DISPUTE_GAME_FACTORY: 0x43EDB88C4B80FDD2ADFF2412A7BEBF9DF42CB40E
OPTIMISM_PORTAL: 0x49048044D57E1C92A77F79988D21FA8FAF74E97E
KAILUA_GAME_TYPE: 1337
```

```admonish warning
Make sure that your `FPVM_IMAGE_ID` matches the value above.
This value determines the exact program used to prove faults.
```

```admonish note
If your `RISC_ZERO_VERIFIER` value is blank, this means that your rollup might be deployed on a base layer that does
not have a deployed RISC Zero zkVM verifier contract.
This means you might have to deploy your own verifier.
Always revise the RISC Zero [documentation](https://dev.risczero.com/api/blockchain-integration/contracts/verifier)
to double-check verifier availability.
```

Once you have these values you'll need to save them for later use during migration.

## Telemetry

All Kailua binaries and commands support exporting telemetry data to an
[OTLP Collector](https://opentelemetry.io/docs/collector/).
The collector endpoint can be specified using the `--otlp-collector` parameter, or through specifying the
`OTLP_COLLECTOR` environment variable.
