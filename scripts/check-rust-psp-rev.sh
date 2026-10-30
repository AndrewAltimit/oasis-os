#!/usr/bin/env bash
# check-rust-psp-rev.sh -- fail unless every PSP toolchain pin agrees with
# the rust-psp revision that CI installs cargo-psp from.
#
# Usage: scripts/check-rust-psp-rev.sh [REV]   (default: $RUST_PSP_REV)
#
# Checks:
#   - For each crates/*/Cargo.toml with a `psp = { ... }` dependency: it is
#     the rust-psp git dependency at exactly REV, the crate's Cargo.lock
#     resolved that revision, and its rust-toolchain.toml pins the nightly
#     docker/psp-build.Dockerfile installs (`ARG NIGHTLY_DATE`).
#   - Every `RUST_PSP_REV:` in .github/workflows/*.yml is REV.
#   - docker/psp-build.Dockerfile's `ARG RUST_PSP_REV` is REV.
#   - The pspdev GCC version the Dockerfile asserts
#     (/opt/pspdev/lib/gcc/psp/<ver>/include) is the one
#     crates/oasis-backend-psp/.cargo/config.toml points bindgen at.

set -euo pipefail

WANT="${1:-${RUST_PSP_REV:-}}"
if [ -z "$WANT" ]; then
    echo "usage: $0 REV (or set RUST_PSP_REV)" >&2
    exit 2
fi

cd "$(dirname "$0")/.."

URL="https://github.com/AndrewAltimit/rust-psp"
DOCKERFILE="docker/psp-build.Dockerfile"
CARGO_CONFIG="crates/oasis-backend-psp/.cargo/config.toml"
errors=0
crates=0

# Value of `ARG NAME=value` in the PSP build Dockerfile (empty if absent).
dockerfile_arg() {
    sed -n "s/^ARG $1=\([^[:space:]]*\).*/\1/p" "$DOCKERFILE" | head -n 1
}

NIGHTLY_DATE=$(dockerfile_arg NIGHTLY_DATE)
if [ -z "$NIGHTLY_DATE" ]; then
    echo "::error file=$DOCKERFILE::no 'ARG NIGHTLY_DATE=...' found"
    errors=$((errors + 1))
fi
CHANNEL_RE="^channel[[:space:]]*=[[:space:]]*\"nightly-$NIGHTLY_DATE\""

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
    elif [ -n "$NIGHTLY_DATE" ] && ! grep -q -E "$CHANNEL_RE" "$dir/rust-toolchain.toml" 2>/dev/null; then
        echo "::error file=$dir/rust-toolchain.toml::channel must be" \
            "\"nightly-$NIGHTLY_DATE\" (ARG NIGHTLY_DATE in $DOCKERFILE)"
        errors=$((errors + 1))
    else
        echo "ok  $dir"
    fi
done

if [ "$crates" -eq 0 ]; then
    echo "::error::no PSP crates found (no crates/*/Cargo.toml with a psp dependency)"
    exit 1
fi

# Every workflow that names the revision must name the same one.
for wf in .github/workflows/*.yml; do
    while IFS= read -r rev; do
        if [ "$rev" != "$WANT" ]; then
            echo "::error file=$wf::RUST_PSP_REV is '$rev', expected '$WANT'"
            errors=$((errors + 1))
        else
            echo "ok  $wf"
        fi
    done < <(sed -n 's/^[[:space:]]*RUST_PSP_REV:[[:space:]]*\([^[:space:]#]*\).*/\1/p' "$wf")
done

docker_rev=$(dockerfile_arg RUST_PSP_REV)
if [ "$docker_rev" != "$WANT" ]; then
    echo "::error file=$DOCKERFILE::ARG RUST_PSP_REV is '${docker_rev:-<none>}', expected '$WANT'"
    errors=$((errors + 1))
else
    echo "ok  $DOCKERFILE"
fi

# The Dockerfile asserts which GCC the pspdev release ships; bindgen's include
# path in the EBOOT's cargo config must name the same version.
GCC_RE='/opt/pspdev/lib/gcc/psp/\([0-9][0-9.]*\)/include'
docker_gcc=$(sed -n "s|.*test -d $GCC_RE.*|\1|p" "$DOCKERFILE" | head -n 1)
config_gcc=$(sed -n "s|^BINDGEN_EXTRA_CLANG_ARGS.*$GCC_RE.*|\1|p" "$CARGO_CONFIG" | head -n 1)
if [ -z "$docker_gcc" ] || [ "$docker_gcc" != "$config_gcc" ]; then
    echo "::error file=$CARGO_CONFIG::bindgen uses pspdev GCC '${config_gcc:-<none>}'" \
        "but $DOCKERFILE checks for '${docker_gcc:-<none>}'"
    errors=$((errors + 1))
else
    echo "ok  pspdev GCC $docker_gcc ($CARGO_CONFIG)"
fi

if [ "$errors" -gt 0 ]; then
    echo "$errors PSP pin mismatch(es) against rust-psp $WANT / nightly-${NIGHTLY_DATE:-?}"
    exit 1
fi
echo "All $crates PSP crates, the workflows and $DOCKERFILE pin rust-psp $WANT" \
    "and nightly-$NIGHTLY_DATE"
