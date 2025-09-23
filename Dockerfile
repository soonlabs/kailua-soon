# TODO alpine is smaller
FROM rust:1.85 as build-environment

ARG CARGO_BUILD_JOBS=16

RUN apt-get update -y && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libclang-dev \
    clang \
    curl \
    cmake \
    ninja-build \
    libssl-dev \
    libc6-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Install svm-rs with reduced parallelism to avoid OOM
RUN CARGO_BUILD_JOBS=1 cargo install svm-rs@0.5.17 --locked

# Install Solana version
RUN svm install 0.8.24

# Install Foundry
RUN curl -L https://foundry.paradigm.xyz | bash && \
    export PATH="$PATH:/root/.foundry/bin" && \
    /root/.foundry/bin/foundryup

WORKDIR /kailua

COPY . .

RUN --mount=type=cache,target=/root/.cargo/registry,sharing=shared \
    --mount=type=cache,target=/root/.cargo/git,sharing=shared \
    --mount=type=cache,target=/kailua/target,sharing=private,id=rust-target-${TARGETARCH} \
    export CC=clang && \
    export CXX=clang++ && \
    export CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_DEBUG=true && \
    export PATH="$PATH:/root/.foundry/bin" && \
    cd crates/contracts/foundry/ && \
    forge build && \
    cd ../../../ && \
    cargo build --jobs ${CARGO_BUILD_JOBS} --release -F disable-dev-mode \
    && mkdir out \
    && mv target/release/kailua-host out/ \
    && mv target/release/kailua-cli out/ \
    && mv target/release/kailua-client out/ \
    && strip out/kailua-host \
    && strip out/kailua-cli \
    && strip out/kailua-client;

FROM rust:1.85 as kailua

# Install bash for the entrypoint script
RUN apt-get update && apt-get install -y bash && rm -rf /var/lib/apt/lists/*

# Copy binaries from build stage
COPY --from=build-environment /kailua/out/kailua-host /usr/local/bin/kailua-host
COPY --from=build-environment /kailua/out/kailua-cli /usr/local/bin/kailua-cli
COPY --from=build-environment /kailua/out/kailua-client /usr/local/bin/kailua-client

# Copy and setup entrypoint script
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Create default data directory
RUN mkdir -p /data

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
