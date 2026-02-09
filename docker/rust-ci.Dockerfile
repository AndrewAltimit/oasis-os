# syntax=docker/dockerfile:1.4
# Rust CI image for oasis_os
# Stable + nightly toolchains with SDL2 dev libs for desktop backend

FROM rust:1.93-slim

# System dependencies (SDL2 dev libs for oasis-backend-sdl)
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    git \
    libsdl2-dev \
    libsdl2-mixer-dev \
    && rm -rf /var/lib/apt/lists/*

# Install nightly toolchain (for format checking with edition 2024)
RUN rustup install nightly \
    && rustup component add rustfmt clippy \
    && rustup component add --toolchain nightly rustfmt

# Install cargo-deny for license/advisory checks
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install cargo-deny --locked 2>/dev/null || true

# Non-root user (overridden by docker-compose USER_ID/GROUP_ID)
RUN useradd -m -u 1000 ciuser \
    && mkdir -p /tmp/cargo && chmod 1777 /tmp/cargo

WORKDIR /workspace

ENV CARGO_HOME=/tmp/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_INCREMENTAL=1 \
    CARGO_NET_RETRY=10 \
    RUST_BACKTRACE=short

CMD ["bash"]
