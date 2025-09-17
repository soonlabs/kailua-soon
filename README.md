# Kailua-Soon

Kailua-Soon bridges the gap between Solana Virtual Machine (SVM) execution and RISC-Zero zkVM proof generation, enabling the first SVM-powered OP Stack rollup with cryptographic validity and fraud proofs. By integrating [kona-soon](../kona-soon)'s revolutionary SVM executor with Kailua's proven zkVM framework, we achieve faster finality and reduced operational costs while maintaining full compatibility with the OP Stack ecosystem.

Unlike traditional EVM-based rollups, Kailua-Soon leverages the high-performance SVM execution environment while preserving the battle-tested OP Stack L2 derivation pipeline, creating a unique hybrid architecture that combines the best of both worlds.

## Architecture Overview

Kailua-Soon serves as the crucial bridge between SVM execution and zkVM proof generation:

### Core Integration Points

1. **SVM Execution Adapter**: Integrates kona-soon's `litesvm` executor into Kailua's proving framework
2. **Enhanced Host Program**: Extended `kailua-host` with SOON network data providers for L2 state verification
3. **Optimized Client Program**: Modified `kailua-client` to handle SVM state transitions and account model proofs
4. **Boundless Market Integration**: Seamless integration with boundless-market v0.13.0 for decentralized proof generation

### Key Improvements

- **🔄 SVM → zkVM Bridge**: First-of-its-kind integration enabling SVM execution proof generation
- **⚡ Enhanced Performance**: Leverages SVM's high-performance execution with zkVM security guarantees
- **🛡️ Decentralized Proving**: Integrated with Boundless proof market for distributed proof generation
- **🌐 OP Stack Compatibility**: Maintains full compatibility with Optimism's Bedrock contracts v1.4.0+

## Development Status

> [!CAUTION]
>
> `Kailua-Soon` is experimental software integrating cutting-edge SVM and zkVM technologies. Not recommended for production usage.

## SVM-Powered Fraud/Validity Proofs

Kailua-Soon enables rollup operators to deploy SVM-based fault proof contracts that leverage RISC-Zero zkVM to generate cryptographic proofs of SVM state transitions. The enhanced `KailuaGame` contract supports:

- **SVM State Validation**: Cryptographic verification of Solana account model state transitions
- **SOON Network Integration**: Native support for SOON network's L2 derivation pipeline
- **Boundless Market Proofs**: Decentralized proof generation through the Boundless ecosystem

## Prerequisites

### Core Dependencies
1. [Rust](https://www.rust-lang.org/tools/install) - Latest stable version
2. [Just](https://just.systems/man/en/) - Command runner for development workflows
3. [Docker](https://www.docker.com/) - Container runtime for local development
4. [Foundry](https://book.getfoundry.sh/getting-started/installation) - Ethereum development toolkit

### SVM Integration Dependencies
5. **kona-soon**: Must be available at `../kona-soon/` (see [kona-soon README](../kona-soon/README.md))
6. **SOON Network Components**: Integrated via workspace dependencies

## Development Workflow

Kailua-Soon provides a comprehensive development workflow for SVM-powered zkVM proving:

### 1. L1 Infrastructure Setup
```bash
just devnet-init-l1      # Initialize beacon chain validator keys
just devnet-build-l1     # Build L1 node and foundry contracts
just devnet-up-l1        # Start L1 (Ethereum) and beacon chain nodes
just devnet-build        # Build l2 image
just devnet-verify       # Verify contract deployment
```

### 2. L2 SOON Network Setup
```bash
just devnet-build-l2     # Build L2 SOON network components
just devnet-up           # Start complete L1+L2 infrastructure
```

### 3. SVM-zkVM Integration
```bash
just devnet-build        # Build kailua-soon with SVM integration
just devnet-upgrade      # Deploy enhanced KailuaGame contracts for SVM
```

### 4. Proof Generation & Validation
```bash
# Launch SVM-aware proposer
just devnet-propose

# Generate a fraud proof
just devnet-fault 1 0

# Launch validator with local proving
just devnet-validate
```


## SVM-zkVM Proof Architecture

Kailua-Soon introduces several key enhancements to support SVM execution within the zkVM proving framework:

### Enhanced Binaries
- **`kailua-client`**: SVM-aware client program that executes state transitions in the zkVM
- **`kailua-host`**: Enhanced host program with SOON network data providers and SVM state management
- **`kailua-cli`**: Command-line interface with SVM-specific operations and Boundless market integration

### Boundless Market Integration
Kailua-Soon seamlessly integrates with the Boundless proof market for decentralized proof generation:

- **Automated Proof Submission**: Validators can automatically submit proofs to the Boundless market
- **Economic Incentives**: Integrated with Boundless market's reward system for proof generators
- **Scalable Proving**: Distribute proof generation across the Boundless network for improved throughput


## Credits & Acknowledgments

Kailua-Soon builds upon the work of several innovative projects:

- **[boundless-xyz/kailua](https://github.com/boundless-xyz/kailua)**: The foundational zkVM proving framework
- **[kona-soon](../kona-soon)**: Revolutionary SVM integration with OP Stack derivation
- **[RISC Zero](https://risczero.com)**: zkVM technology enabling verifiable SVM execution
- **[Boundless](https://boundless.market)**: Decentralized proof market for scalable verification
- **[SOON Network](https://soo.network)**: SVM-powered L2 infrastructure


## License

Licensed under the [Apache License 2.0](LICENSE)

> [!NOTE]
>
> Contributions intentionally submitted for inclusion in this project by you
> shall be licensed as above, without any additional terms or conditions.
