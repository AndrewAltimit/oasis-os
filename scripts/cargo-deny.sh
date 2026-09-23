#!/usr/bin/env bash
# cargo-deny.sh -- Run `cargo deny check` robustly against a shared CARGO_HOME.
#
# Usage:
#   scripts/cargo-deny.sh [cargo deny check args...]
#
# CI mounts a persistent named volume at CARGO_HOME (/tmp/cargo), so the
# RustSec advisory-db git checkout under $CARGO_HOME/advisory-dbs outlives
# each container. When a run is cancelled or times out mid-fetch, git leaves
# `.git/*.lock` files behind and every later run fails with
# "cannot lock ref 'HEAD': ... HEAD.lock: File exists". This script:
#   1. serializes advisory-db access across concurrent CI jobs with flock,
#   2. removes stale git lock files (safe: we hold the lock, so no other
#      cargo-deny can be fetching),
#   3. if the check still fails on a fetch error, wipes the advisory-db
#      checkout and retries once from a fresh clone.

set -euo pipefail

CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
DB_ROOT="$CARGO_HOME/advisory-dbs"
mkdir -p "$DB_ROOT"

exec 9>"$DB_ROOT/.oasis-ci.lock"
flock -w 600 9

clear_stale_locks() {
  find "$DB_ROOT" -path '*/.git/*' -name '*.lock' -type f -print -delete || true
}

clear_stale_locks

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

if cargo deny check "$@" 2>&1 | tee "$log"; then
  exit 0
fi

if ! grep -q 'failed to fetch advisory database' "$log"; then
  exit 1
fi

echo "cargo-deny: advisory-db fetch failed; wiping checkout and retrying" >&2
find "$DB_ROOT" -mindepth 1 -maxdepth 1 -type d -exec rm -rf {} +
cargo deny check "$@"
