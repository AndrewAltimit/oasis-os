# syntax=docker/dockerfile:1.4
# PSP build environment for oasis_os: reproduces the CI "Build PSP ..." steps
# (the CI runner is self-hosted and has these tools preinstalled).
#
#   - pspdev toolchain at /opt/pspdev (psp-gcc 15.2.0, psp-ld): QuickJS-NG's C
#     sources are cross-compiled with it and the EBOOT links through psp-ld
#     (see crates/oasis-backend-psp/.cargo/config.toml).
#   - libclang for rquickjs-sys's bindgen.
#   - The nightly every PSP crate pins in its rust-toolchain.toml, + rust-src.
#   - cargo-psp (+ prxgen, pack-pbp, mksfo, prxmin) from the pinned rust-psp
#     revision, the same way .github/actions/install-cargo-psp does.
#
# Usage (from the repo root, PowerShell or a POSIX shell):
#   docker compose --profile psp-build build psp-build
#   docker compose --profile psp-build run --rm psp-build \
#     bash -c 'cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release'
#
# Bump NIGHTLY_DATE / RUST_PSP_REV together with the crates' rust-toolchain.toml
# and RUST_PSP_REV in .github/workflows/*.yml. If PSPDEV_RELEASE ships a new
# GCC, update the `test -d` path below and BINDGEN_EXTRA_CLANG_ARGS in
# crates/oasis-backend-psp/.cargo/config.toml. scripts/check-rust-psp-rev.sh
# (run by CI) fails if any of these drift apart.

FROM rust:1.99-slim

ARG NIGHTLY_DATE=2026-04-13
ARG RUST_PSP_REV=e4cc06f6fa50fa45dc3ae8b332e8a2b67625f588
ARG PSPDEV_RELEASE=v20261001

# Runtime libraries the prebuilt pspdev binaries link against, plus the
# tools the build needs (python3 runs tools/fix-symtab-shinfo.py).
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git make pkg-config python3 xz-utils bzip2 \
    clang libclang-dev \
    libgmp10 libmpfr6 libmpc3 libusb-0.1-4 \
    libreadline8t64 libcurl4t64 libarchive13t64 libssl3t64 \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/pspdev.tar.gz \
    "https://github.com/pspdev/pspdev/releases/download/${PSPDEV_RELEASE}/pspdev-debian-latest.tar.gz" \
    && tar xzf /tmp/pspdev.tar.gz -C /opt \
    && rm /tmp/pspdev.tar.gz \
    && /opt/pspdev/bin/psp-gcc --version | head -1 \
    && test -d /opt/pspdev/lib/gcc/psp/15.2.0/include

RUN rustup toolchain install "nightly-${NIGHTLY_DATE}" --profile minimal \
    --component rust-src \
    && rustc "+nightly-${NIGHTLY_DATE}" -vV

RUN cargo install --locked --root /opt/cargo-psp \
    --git https://github.com/AndrewAltimit/rust-psp --rev "${RUST_PSP_REV}" cargo-psp \
    && rm -rf /usr/local/cargo/registry /usr/local/cargo/git

RUN useradd -m -u 1000 ciuser \
    && mkdir -p /tmp/cargo && chmod 1777 /tmp/cargo \
    && git config --system --add safe.directory '*'

ENV PSPDEV=/opt/pspdev \
    PATH=/opt/cargo-psp/bin:/opt/pspdev/bin:$PATH \
    CARGO_HOME=/tmp/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_NET_RETRY=10 \
    RUST_BACKTRACE=short

WORKDIR /app
CMD ["bash"]
