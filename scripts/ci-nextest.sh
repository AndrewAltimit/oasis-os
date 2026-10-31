#!/usr/bin/env bash
# Run the workspace test suite as CI does (main-ci.yml and pr-validation.yml
# call this inside the rust-ci container), retrying tests that only timed out.
#
# 1. `cargo nextest run --workspace --profile ci`.
# 2. If every failure was a TIMEOUT (the hang guard in .config/nextest.toml
#    killed a test that ran too long), rerun just those tests, alone, on an
#    otherwise idle pool. A test that passes then was slow under the loaded
#    pool, not broken: it is reported as FLAKY (a GitHub warning) and the
#    run passes. A test that times out again fails the run.
#    Any other failure (assertion, panic, crash, build error) fails the run
#    without a retry.
# 3. Doc-tests (`cargo test --workspace --doc`; nextest doesn't run them).
#
# Usage: scripts/ci-nextest.sh [extra nextest args...]
set -uo pipefail

LOG=$(mktemp)
trap 'rm -f "$LOG"' EXIT

strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }

cargo nextest run --workspace --profile ci "$@" 2>&1 | tee "$LOG"
status=${PIPESTATUS[0]}

if [ "$status" -ne 0 ]; then
    plain=$(strip_ansi < "$LOG")
    summary=$(grep -E '^ *Summary \[' <<<"$plain" | tail -1)
    # `TIMEOUT [ 600.040s] (9502/9502) oasis-app::e2e_soak some_test`:
    # binary id, then test name. nextest prints each one twice (live and
    # in the final summary), hence sort -u.
    timed_out=$(grep -E '^ *TIMEOUT \[' <<<"$plain" \
        | sed -E 's/^ *TIMEOUT \[[^]]*\] (\([^)]*\) )?//' | sort -u)
    # Without a summary line the run never got to the tests (build error).
    # A summary counting anything but timeouts as failed is a real failure.
    if [ -z "$summary" ] || [ -z "$timed_out" ] \
        || grep -qE '[0-9]+ (failed|exec failed|leaky failed)' <<<"$summary"; then
        echo "Test run failed (not only timeouts); no retry."
        exit "$status"
    fi

    echo
    echo "Only timeouts failed; retrying them alone:"
    echo "$timed_out" | sed 's/^/  /'
    filter=""
    while read -r bin name; do
        [ -n "$bin" ] || continue
        expr="(binary_id(=$bin) & test(=$name))"
        filter=${filter:+$filter | }$expr
    done <<<"$timed_out"

    if ! cargo nextest run --workspace --profile ci -E "$filter"; then
        echo "::error::Timed-out tests failed again on retry:" $timed_out
        exit 1
    fi
    while read -r bin name; do
        [ -n "$bin" ] || continue
        echo "FLAKY (timed out under load, passed on retry): $bin $name"
        echo "::warning title=Flaky test::$bin $name timed out in the full run" \
            "and passed when retried alone"
    done <<<"$timed_out"
fi

cargo test --workspace --doc
