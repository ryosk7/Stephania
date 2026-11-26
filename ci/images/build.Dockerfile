FROM ubuntu:24.04
ARG RUST_VERSION=1.96.0
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:$PATH
RUN set -eu; for attempt in 1 2 3 4 5; do \
      if apt-get -o Acquire::Retries=3 -o APT::Update::Error-Mode=any update && apt-get -o Acquire::Retries=3 install -y --no-install-recommends ca-certificates curl build-essential pkg-config libasound2-dev libclang-dev libx264-dev libavcodec-dev libavutil-dev git python3 bash time sccache; then rm -rf /var/lib/apt/lists/*; exit 0; fi; \
      rm -rf /var/lib/apt/lists/*; if [ "$attempt" -lt 5 ]; then sleep "$((attempt * 5))"; fi; \
    done; exit 1
RUN curl --retry 5 --retry-all-errors -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION" --component clippy
RUN chmod -R a+rX /opt/rustup /opt/cargo && rustc --version && cargo --version && cargo clippy --version
