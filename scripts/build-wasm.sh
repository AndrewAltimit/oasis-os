#!/usr/bin/env bash
# Build the OASIS_OS WASM backend with wasm-pack.
#
# Output goes to ./pkg/ (wasm-pack default for --target web).
# Serve ./www/ with any static file server; index.js imports from ../pkg/.
#
# Usage:
#   ./scripts/build-wasm.sh          # debug build
#   ./scripts/build-wasm.sh --release # release (smaller + faster)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROFILE="${1:-}"

if [ "$PROFILE" = "--release" ]; then
  echo "==> Building WASM (release)..."
  wasm-pack build crates/oasis-backend-wasm --target web --release --out-dir "$ROOT/pkg"
else
  echo "==> Building WASM (debug)..."
  wasm-pack build crates/oasis-backend-wasm --target web --dev --out-dir "$ROOT/pkg"
fi

echo "==> Build complete. Output in pkg/"
echo ""
echo "To run locally:"
echo "  python3 -m http.server 8080"
echo "  open http://localhost:8080/www/"
