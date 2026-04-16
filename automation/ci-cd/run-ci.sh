#!/bin/bash
# CI stage runner for oasis-os
#
# Provides the same interface as template-repo's run-ci.sh so that
# automation-cli review respond/failure/precommit can call CI stages
# generically. All Rust operations run inside Docker (rust-ci service).
#
# Usage: ./automation/ci-cd/run-ci.sh <stage> [extra-args...]
#
# Stages:
#   autoformat    - cargo fmt --all (format in-place, no --check)
#   format        - cargo fmt --all -- --check
#   lint-basic    - cargo clippy --workspace -- -D warnings
#   lint-full     - cargo clippy --workspace -- -D warnings (same as lint-basic)
#   test          - cargo test --workspace
#   build         - cargo build --workspace --release
#   deny          - cargo deny check
#   full          - format + lint-basic + test + build + deny

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

# Ensure UID/GID are set for Docker
export USER_ID="${USER_ID:-$(id -u)}"
export GROUP_ID="${GROUP_ID:-$(id -g)}"

STAGE="${1:-format}"
shift 2>/dev/null || true

DOCKER_RUN="docker compose --profile ci run --rm rust-ci"

run_format_check() {
    $DOCKER_RUN cargo fmt --all -- --check
}

run_autoformat() {
    $DOCKER_RUN cargo fmt --all
}

run_lint_basic() {
    $DOCKER_RUN cargo clippy --workspace -- -D warnings
}

run_test() {
    $DOCKER_RUN cargo test --workspace
}

run_build() {
    $DOCKER_RUN cargo build --workspace --release
}

run_deny() {
    $DOCKER_RUN cargo deny check
}

case "$STAGE" in
    autoformat)
        run_autoformat
        ;;
    format)
        run_format_check
        ;;
    lint-basic|lint-full|lint)
        run_lint_basic
        ;;
    test)
        run_test
        ;;
    build)
        run_build
        ;;
    deny)
        run_deny
        ;;
    full)
        echo "=== Format Check ==="
        run_format_check
        echo "=== Clippy ==="
        run_lint_basic
        echo "=== Tests ==="
        run_test
        echo "=== Build ==="
        run_build
        echo "=== Deny ==="
        run_deny
        ;;
    *)
        echo "Unknown stage: $STAGE" >&2
        echo "Available: autoformat, format, lint-basic, lint-full, test, build, deny, full" >&2
        exit 1
        ;;
esac
