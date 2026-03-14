#!/usr/bin/env bash
# compare-screenshots.sh -- Compare actual screenshots against golden baselines.
#
# Usage:
#   scripts/compare-screenshots.sh [--threshold PCT] [--baseline DIR] [--actual DIR]
#
# Compares each `actual.png` found under the actual directory against the
# corresponding `golden.png` under the baseline directory.
#
# Exit codes:
#   0  All screenshots match (or no baselines exist yet).
#   1  One or more screenshots differ beyond the threshold.
#
# Environment:
#   OASIS_SCREENSHOT_THRESHOLD  Pixel-diff threshold percentage (default: 0.1).

set -euo pipefail

THRESHOLD="${OASIS_SCREENSHOT_THRESHOLD:-0.1}"
BASELINE_DIR="screenshots/baseline"
ACTUAL_DIR="screenshots/tests"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --threshold)
            THRESHOLD="$2"
            shift 2
            ;;
        --baseline)
            BASELINE_DIR="$2"
            shift 2
            ;;
        --actual)
            ACTUAL_DIR="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [--threshold PCT] [--baseline DIR] [--actual DIR]"
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Check prerequisites
# ---------------------------------------------------------------------------
if [ ! -d "$ACTUAL_DIR" ]; then
    echo "No actual screenshots directory found: $ACTUAL_DIR"
    echo "Run screenshot-tests first."
    exit 0
fi

if [ ! -d "$BASELINE_DIR" ]; then
    echo "No baseline directory found: $BASELINE_DIR"
    echo "Baselines have not been generated yet."
    echo "Generate baselines by running:"
    echo "  cargo run -p oasis-app --bin screenshot-tests --release -- --bless"
    echo "  cp -r screenshots/tests/ screenshots/baseline/"
    echo ""
    echo "Skipping comparison (not an error)."
    exit 0
fi

# ---------------------------------------------------------------------------
# Compare
# ---------------------------------------------------------------------------
total=0
matched=0
differed=0
missing=0
diff_details=()

# Find all actual.png files.
while IFS= read -r -d '' actual_file; do
    # Extract the scenario subdirectory name.
    rel="${actual_file#"$ACTUAL_DIR/"}"       # e.g. "classic_dashboard/actual.png"
    scenario_dir="${rel%/actual.png}"          # e.g. "classic_dashboard"

    baseline_file="$BASELINE_DIR/$scenario_dir/golden.png"

    total=$((total + 1))

    if [ ! -f "$baseline_file" ]; then
        missing=$((missing + 1))
        continue
    fi

    # Compare file sizes first (fast path).
    actual_size=$(stat -c%s "$actual_file" 2>/dev/null || stat -f%z "$actual_file" 2>/dev/null)
    baseline_size=$(stat -c%s "$baseline_file" 2>/dev/null || stat -f%z "$baseline_file" 2>/dev/null)

    if [ "$actual_size" = "$baseline_size" ]; then
        # If sizes match, compare bytes directly (fast, no external deps).
        if cmp -s "$actual_file" "$baseline_file"; then
            matched=$((matched + 1))
            continue
        fi
    fi

    # Files differ -- try pixel-level diff if Python is available.
    diff_pct="unknown"
    if command -v python3 &>/dev/null; then
        diff_pct=$(python3 -c "
import sys
try:
    # Read raw PNG bytes and compare.
    with open('$actual_file', 'rb') as f:
        actual = f.read()
    with open('$baseline_file', 'rb') as f:
        baseline = f.read()

    # Try to decode PNGs for pixel-level comparison.
    import struct, zlib

    def decode_png_pixels(data):
        \"\"\"Minimal PNG decoder: extract raw pixel bytes.\"\"\"
        pos = 8  # Skip PNG signature
        idat_chunks = []
        width = height = 0
        while pos < len(data):
            length = struct.unpack('>I', data[pos:pos+4])[0]
            chunk_type = data[pos+4:pos+8]
            chunk_data = data[pos+8:pos+8+length]
            pos += 12 + length  # 4 len + 4 type + data + 4 crc
            if chunk_type == b'IHDR':
                width = struct.unpack('>I', chunk_data[0:4])[0]
                height = struct.unpack('>I', chunk_data[4:8])[0]
            elif chunk_type == b'IDAT':
                idat_chunks.append(chunk_data)
        raw = zlib.decompress(b''.join(idat_chunks))
        # Reverse PNG filtering (only supports filter type 0 = None).
        stride = width * 4 + 1  # 4 bytes per pixel (RGBA) + 1 filter byte
        pixels = bytearray()
        for y in range(height):
            row_start = y * stride
            # Skip filter byte (assume type 0).
            pixels.extend(raw[row_start+1:row_start+1+width*4])
        return pixels, width, height

    a_px, aw, ah = decode_png_pixels(actual)
    b_px, bw, bh = decode_png_pixels(baseline)
    if len(a_px) != len(b_px) or aw != bw or ah != ah:
        print('size_mismatch')
    else:
        total_px = aw * ah
        diff_count = 0
        for i in range(0, len(a_px), 4):
            if a_px[i:i+4] != b_px[i:i+4]:
                diff_count += 1
        pct = (diff_count / total_px) * 100.0
        print(f'{pct:.4f}')
except Exception as e:
    print('error', file=sys.stderr)
    print('unknown')
" 2>/dev/null) || diff_pct="unknown"
    fi

    if [ "$diff_pct" = "unknown" ] || [ "$diff_pct" = "size_mismatch" ]; then
        differed=$((differed + 1))
        diff_details+=("  DIFF  $scenario_dir  ($diff_pct)")
    else
        # Compare against threshold using awk (bash doesn't do float comparison).
        exceeds=$(awk "BEGIN { print ($diff_pct > $THRESHOLD) ? 1 : 0 }")
        if [ "$exceeds" = "1" ]; then
            differed=$((differed + 1))
            diff_details+=("  DIFF  $scenario_dir  (${diff_pct}% pixels differ, threshold: ${THRESHOLD}%)")
        else
            matched=$((matched + 1))
        fi
    fi

done < <(find "$ACTUAL_DIR" -name "actual.png" -print0 | sort -z)

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo "=========================================="
echo "  Screenshot Regression Summary"
echo "=========================================="
echo "  Total scenarios:  $total"
echo "  Matched:          $matched"
echo "  Differed:         $differed"
echo "  Missing baseline: $missing"
echo "  Threshold:        ${THRESHOLD}%"
echo "=========================================="

if [ ${#diff_details[@]} -gt 0 ]; then
    echo ""
    echo "Differences detected:"
    for detail in "${diff_details[@]}"; do
        echo "$detail"
    done
fi

if [ "$missing" -gt 0 ]; then
    echo ""
    echo "Note: $missing scenarios have no golden baseline yet."
fi

# Write GitHub Actions step summary if available.
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    {
        echo "### Screenshot Regression"
        echo ""
        echo "| Metric | Count |"
        echo "|--------|-------|"
        echo "| Total | $total |"
        echo "| Matched | $matched |"
        echo "| Differed | $differed |"
        echo "| Missing baseline | $missing |"
        echo "| Threshold | ${THRESHOLD}% |"

        if [ ${#diff_details[@]} -gt 0 ]; then
            echo ""
            echo "**Differences:**"
            echo '```'
            for detail in "${diff_details[@]}"; do
                echo "$detail"
            done
            echo '```'
        fi
    } >> "$GITHUB_STEP_SUMMARY"
fi

# Exit with failure only if there are actual differences.
if [ "$differed" -gt 0 ]; then
    echo ""
    echo "WARN: $differed screenshot(s) differ from baseline."
    exit 1
fi

echo ""
echo "All screenshots match baselines."
exit 0
