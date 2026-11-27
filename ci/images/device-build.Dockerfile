# Build environment for the binaries that go into the OS image: the same
# distribution as the device (Raspberry Pi OS = Debian 13 trixie), so the
# engine links against the sonames the image ships (libavcodec61,
# libavutil59, libx264-164, libasound2t64). ci/images/build.Dockerfile
# (Ubuntu 24.04) stays the environment for tests on the CI runners.
FROM debian:trixie
ARG RUST_VERSION=1.96.0
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:$PATH
RUN set -eu; for attempt in 1 2 3 4 5; do \
      if apt-get -o Acquire::Retries=3 -o APT::Update::Error-Mode=any update && apt-get -o Acquire::Retries=3 install -y --no-install-recommends ca-certificates curl build-essential pkg-config libasound2-dev libclang-dev libx264-dev libavcodec-dev libavutil-dev git python3 bash time; then rm -rf /var/lib/apt/lists/*; exit 0; fi; \
      rm -rf /var/lib/apt/lists/*; if [ "$attempt" -lt 5 ]; then sleep "$((attempt * 5))"; fi; \
    done; exit 1
# sccache when the distribution has it; otherwise a pass-through wrapper so
# the shared cached-build action (RUSTC_WRAPPER=sccache) works unchanged.
RUN set -eu; if apt-get update && apt-get install -y --no-install-recommends sccache; then :; else \
      printf '#!/bin/sh\ncase "$1" in --*) exit 0;; esac\nexec "$@"\n' >/usr/local/bin/sccache && chmod 0755 /usr/local/bin/sccache; fi; \
    rm -rf /var/lib/apt/lists/*
RUN curl --retry 5 --retry-all-errors -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION"
RUN chmod -R a+rX /opt/rustup /opt/cargo && rustc --version && cargo --version
