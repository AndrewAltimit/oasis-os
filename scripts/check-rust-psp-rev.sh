#!/usr/bin/env bash
# check-rust-psp-rev.sh -- fail unless every PSP crate pins the same rust-psp
# revision that CI installs cargo-psp from.
#
# Usage: scripts/check-rust-psp-rev.sh [REV]   (default: $RUST_PSP_REV)
#
# For each crates/*/Cargo.toml with a `psp = { ... }` dependency, checks that
# it is the rust-psp git dependency at exactly REV and that the crate's
# Cargo.lock resolved that revision.

set -euo pipefail

WANT="${1:-${RUST_PSP_REV:-}}"
if [ -z "$WANT" ]; then
    echo "usage: $0 REV (or set RUST_PSP_REV)" >&2
    exit 2
fi

cd "$(dirname "$0")/.."

URL="https://github.com/AndrewAltimit/rust-psp"
errors=0
crates=0
for manifest in crates/*/Cargo.toml; do
    line=$(grep -E '^psp[[:space:]]*=[[:space:]]*\{' "$manifest" || true)
    [ -n "$line" ] || continue
    crates=$((crates + 1))
    dir=$(dirname "$manifest")
    rev=$(sed -n 's/.*rev[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' <<<"$line")
    if ! grep -q -F "git = \"$URL\"" <<<"$line"; then
        echo "::error file=$manifest::psp must be a git dependency on $URL: $line"
        errors=$((errors + 1))
    elif [ "$rev" != "$WANT" ]; then
        echo "::error file=$manifest::psp rev is '${rev:-<none>}', expected '$WANT'"
        errors=$((errors + 1))
    elif ! grep -q -F "source = \"git+$URL?rev=$WANT#$WANT\"" "$dir/Cargo.lock" 2>/dev/null; then
        echo "::error file=$dir/Cargo.lock::Cargo.lock does not resolve psp at $WANT (run cargo update -p psp)"
        errors=$((errors + 1))
    else
        echo "ok  $dir"
    fi
done

if [ "$crates" -eq 0 ]; then
    echo "::error::no PSP crates found (no crates/*/Cargo.toml with a psp dependency)"
    exit 1
fi
if [ "$errors" -gt 0 ]; then
    echo "$errors of $crates PSP crate(s) do not pin rust-psp $WANT"
    exit 1
fi
echo "All $crates PSP crates pin rust-psp $WANT"
