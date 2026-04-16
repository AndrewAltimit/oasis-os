#!/usr/bin/env bash
# Compare criterion benchmark results between a baseline and the current run.
# Outputs a markdown summary suitable for $GITHUB_STEP_SUMMARY.
#
# Usage: bench-compare.sh <baseline-dir> <current-dir> [threshold-pct]
#
# baseline-dir: path to criterion output from main branch (e.g. baseline-criterion/)
# current-dir:  path to criterion output from the PR (e.g. target/criterion/)
# threshold-pct: regression warning threshold in percent (default: 10)

set -euo pipefail

BASELINE_DIR="${1:?Usage: bench-compare.sh <baseline-dir> <current-dir> [threshold-pct]}"
CURRENT_DIR="${2:?Usage: bench-compare.sh <baseline-dir> <current-dir> [threshold-pct]}"
THRESHOLD="${3:-10}"

if [ ! -d "$BASELINE_DIR" ]; then
  echo "No baseline directory found at $BASELINE_DIR -- skipping comparison."
  exit 0
fi

if [ ! -d "$CURRENT_DIR" ]; then
  echo "No current benchmark directory found at $CURRENT_DIR -- skipping comparison."
  exit 0
fi

HAS_REGRESSION=0
HAS_ANY=0

# Header
echo "### Benchmark Comparison (vs main)"
echo ""
echo "| Benchmark | Baseline (ns) | Current (ns) | Change | Status |"
echo "|-----------|--------------|-------------|--------|--------|"

# Find all estimate files in the current run
while IFS= read -r -d '' current_est; do
  # Extract the benchmark path relative to criterion dir
  # Structure: <dir>/criterion/<group>/<bench>/new/estimates.json
  rel_path="${current_est#"$CURRENT_DIR"/}"

  # Corresponding baseline file
  baseline_est="$BASELINE_DIR/$rel_path"

  if [ ! -f "$baseline_est" ]; then
    continue
  fi

  # Extract the benchmark name from the path (group/bench)
  # rel_path looks like: group_name/bench_name/new/estimates.json
  bench_name=$(echo "$rel_path" | sed 's|/new/estimates.json||; s|/| / |g')

  # Parse point estimates (median, in nanoseconds)
  baseline_ns=$(python3 -c "
import json, sys
try:
    d = json.load(open('$baseline_est'))
    print(d.get('median', d.get('mean', {})).get('point_estimate', 0))
except Exception:
    print(0)
" 2>/dev/null || echo "0")

  current_ns=$(python3 -c "
import json, sys
try:
    d = json.load(open('$current_est'))
    print(d.get('median', d.get('mean', {})).get('point_estimate', 0))
except Exception:
    print(0)
" 2>/dev/null || echo "0")

  # Skip if either is zero/missing
  if python3 -c "exit(0 if float('$baseline_ns') > 0 and float('$current_ns') > 0 else 1)" 2>/dev/null; then
    :
  else
    continue
  fi

  HAS_ANY=1

  # Calculate percentage change
  pct_change=$(python3 -c "
b = float('$baseline_ns')
c = float('$current_ns')
pct = ((c - b) / b) * 100.0
print(f'{pct:+.1f}%')
")

  pct_num=$(python3 -c "
b = float('$baseline_ns')
c = float('$current_ns')
print(((c - b) / b) * 100.0)
")

  # Format nanoseconds for readability
  fmt_baseline=$(python3 -c "print(f'{float(\"$baseline_ns\"):.0f}')")
  fmt_current=$(python3 -c "print(f'{float(\"$current_ns\"):.0f}')")

  # Determine status
  is_regression=$(python3 -c "print('yes' if float('$pct_num') > float('$THRESHOLD') else 'no')")
  is_improvement=$(python3 -c "print('yes' if float('$pct_num') < -float('$THRESHOLD') else 'no')")

  if [ "$is_regression" = "yes" ]; then
    status="**REGRESSION**"
    HAS_REGRESSION=1
  elif [ "$is_improvement" = "yes" ]; then
    status="improvement"
  else
    status="ok"
  fi

  echo "| $bench_name | $fmt_baseline | $fmt_current | $pct_change | $status |"

done < <(find "$CURRENT_DIR" -path "*/new/estimates.json" -print0 2>/dev/null | sort -z)

if [ "$HAS_ANY" -eq 0 ]; then
  echo ""
  echo "*No comparable benchmarks found between baseline and current run.*"
fi

echo ""

if [ "$HAS_REGRESSION" -eq 1 ]; then
  echo "> **Warning**: One or more benchmarks regressed by more than ${THRESHOLD}%."
  echo "> This is advisory only and does not block the PR."
fi

# Gate mode: exit 1 on regression when BENCH_GATE=1
if [ "$HAS_REGRESSION" -eq 1 ] && [ "${BENCH_GATE:-0}" = "1" ]; then
  echo "> Benchmark gate FAILED: regressions exceed ${THRESHOLD}% threshold."
  exit 1
fi

exit 0
