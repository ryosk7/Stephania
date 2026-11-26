FROM ubuntu:24.04
ARG RUST_VERSION=1.96.0
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl build-essential pkg-config libasound2-dev libclang-dev libx264-dev libavcodec-dev libavutil-dev git python3 bash time sccache && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION" --component clippy
RUN chmod -R a+rX /opt/rustup /opt/cargo && rustc --version && cargo --version && cargo clippy --version
