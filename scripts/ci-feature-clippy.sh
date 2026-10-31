#!/usr/bin/env bash
# ci-feature-clippy.sh -- check and lint the supported non-default feature sets.
#
# Usage:
#   scripts/ci-feature-clippy.sh          # run every check (inside rust-ci)
#   scripts/ci-feature-clippy.sh --list   # print the commands, run nothing
#
# The workspace clippy only sees one feature resolution: feature unification
# gives oasis-browser its defaults + javascript, oasis-video demux-lite +
# ffmpeg, and oasis-app its defaults (video-decode-ffmpeg). Every other
# supported configuration below has broken main at least once while the
# workspace clippy stayed green (a `_video`-only field read without a
# decoder, a test re-export unused without `native_net`, ...).
#
# main-ci.yml and pr-validation.yml both call this script, so the two
# workflows cannot drift. Each line is a single-crate cargo invocation, so
# dependencies whose features don't change are reused from the workspace
# build that ran earlier in the same job.
#
# Every command runs even after a failure, so one CI run reports all broken
# feature sets; the script exits non-zero if any of them failed.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 2

CHECKS=(
    # oasis-app without a video decoder, with and without the MCP server.
    "check -p oasis-app --no-default-features --features javascript"
    "check -p oasis-app --no-default-features --features javascript,mcp"
)

CLIPPIES=(
    "-p oasis-browser --features parallel-style,webp"
    "-p oasis-browser --no-default-features"
    "-p oasis-browser --no-default-features --features psp,javascript"
    "-p oasis-core --no-default-features"
    "-p oasis-video"
    "-p oasis-video --features demux-lite"
    "-p oasis-video --features symphonia"
    "-p oasis-video --features h264"
    "-p oasis-video --all-features"
    "-p oasis-app --no-default-features --features video-decode"
    "-p oasis-ffi --features video-decode"
    "-p oasis-ffi --features video-decode-ffmpeg"
    # The glow GPU shader path: no in-tree host enables it, so nothing else
    # compiles it.
    "-p oasis-shader --features gl-native"
    # WASM without the YouTube app (and so oasis-apps without video-embed,
    # which the workspace build always unifies on).
    "-p oasis-backend-wasm --no-default-features"
)

commands=()
for args in "${CHECKS[@]}"; do
    commands+=("cargo $args")
done
for args in "${CLIPPIES[@]}"; do
    commands+=("cargo clippy --all-targets $args -- -D warnings")
done

if [ "${1:-}" = "--list" ]; then
    printf '%s\n' "${commands[@]}"
    exit 0
fi

# GitHub Actions folds ::group:: blocks; elsewhere they are plain markers.
failed=()
for cmd in "${commands[@]}"; do
    echo "::group::$cmd"
    start=$SECONDS
    # Word splitting is intended: every entry is a plain argument list.
    # shellcheck disable=SC2086
    if $cmd; then
        status=ok
    else
        status=FAILED
        failed+=("$cmd")
    fi
    echo "::endgroup::"
    echo "$status ($((SECONDS - start))s): $cmd"
done

if [ "${#failed[@]}" -gt 0 ]; then
    echo
    echo "${#failed[@]} of ${#commands[@]} feature-set checks failed:"
    for cmd in "${failed[@]}"; do
        echo "::error::feature-set check failed: $cmd"
    done
    exit 1
fi
echo "All ${#commands[@]} feature-set checks passed."
