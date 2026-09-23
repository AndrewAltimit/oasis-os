#!/usr/bin/env bash
# run-release-bin.sh -- Run a prebuilt target/release binary that links SDL3.
#
# Usage:
#   scripts/run-release-bin.sh <binary> [args...]
#
# The `sdl3` crate's `build-from-source` feature produces a shared
# libSDL3.so.0 under target/release/build/sdl3-sys-*/out/lib and the
# binaries carry no rpath to it. `cargo run` puts that directory on
# LD_LIBRARY_PATH automatically; running the binary directly does not, so
# it fails with "libSDL3.so.0: cannot open shared object file". This script
# does what `cargo run` would, without cargo re-resolving features and
# rebuilding the workspace.

set -euo pipefail

if [ $# -lt 1 ]; then
  echo "usage: $0 <binary> [args...]" >&2
  exit 2
fi

BIN="$1"
shift

# Newest first: a persistent target dir can hold several sdl3-sys hashes.
SDL_LIB=$(find target/release/build -path '*/sdl3-sys-*/out/lib/libSDL3.so.0' \
  -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -n 1 | cut -d' ' -f2- || true)
if [ -z "$SDL_LIB" ]; then
  echo "error: libSDL3.so.0 not found under target/release/build/sdl3-sys-*/out/lib" >&2
  echo "       run 'cargo build --workspace --release' first" >&2
  exit 1
fi

SDL_DIR=$(dirname "$SDL_LIB")
export LD_LIBRARY_PATH="$SDL_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "./target/release/$BIN" "$@"
