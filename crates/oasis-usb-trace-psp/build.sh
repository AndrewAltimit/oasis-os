#!/usr/bin/env bash
# Build USB Host Phase 0b kernel MMIO trace EBOOT.PBP for PSP.
#
# This is a KERNEL-MODE EBOOT. Requires custom firmware (PRO-C2 / ARK-4).
#
# Prerequisites:
#   rustup -- the nightly pinned in rust-toolchain.toml (with rust-src) is
#     installed automatically on the first build.
#   cargo-psp from the rust-psp fork (crates.io `cargo-psp` is upstream's),
#     built from the revision the `psp` dependency in Cargo.toml pins:
#     cargo install --git https://github.com/AndrewAltimit/rust-psp \
#       --rev <rev from Cargo.toml> cargo-psp --locked
#
# Usage:
#   ./build.sh          # debug build
#   ./build.sh release  # release build (optimized)

set -euo pipefail
cd "$(dirname "$0")"

export RUST_PSP_BUILD_STD=1

if [ "${1:-}" = "release" ]; then
    cargo psp --release
    echo ""
    echo "EBOOT.PBP: target/mipsel-sony-psp-std/release/EBOOT.PBP"
else
    cargo psp
    echo ""
    echo "EBOOT.PBP: target/mipsel-sony-psp-std/debug/EBOOT.PBP"
fi

echo "Copy to: ms0:/PSP/GAME/USBTRACE/EBOOT.PBP"
