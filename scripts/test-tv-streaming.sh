#!/usr/bin/env bash
# Test script for TV Guide streaming video decode.
#
# Launches oasis-app with auto-launch TV Guide + auto-tune channel,
# waits for video decode results, and reports success/failure.
#
# Usage:
#   ./scripts/test-tv-streaming.sh              # default: CH2, 15s timeout
#   ./scripts/test-tv-streaming.sh 4 20         # CH4, 20s timeout
#   ./scripts/test-tv-streaming.sh 2 15 xdotool # also send xdotool clicks

set -euo pipefail

CHANNEL=${1:-2}
TIMEOUT=${2:-15}
USE_XDOTOOL=${3:-}
LOGFILE=$(mktemp /tmp/oasis-tv-test-XXXXXX.log)

echo "=== TV Streaming Decode Test ==="
echo "Channel: $CHANNEL | Timeout: ${TIMEOUT}s | Log: $LOGFILE"
echo ""

# Build
echo "Building..."
cargo build --release -p oasis-app 2>&1 | tail -1

# Launch with auto-tune env vars
echo "Launching oasis-app..."
OASIS_APP="TV Guide" \
OASIS_TV_CHANNEL="$CHANNEL" \
OASIS_TV_TIMEOUT="$TIMEOUT" \
  cargo run --release -p oasis-app 2>"$LOGFILE" &
APP_PID=$!

cleanup() {
    if [[ -n "${APP_PID:-}" ]] && kill -0 "$APP_PID" 2>/dev/null; then
        kill "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Optional: send additional xdotool clicks for navigation testing
if [[ "$USE_XDOTOOL" == "xdotool" ]]; then
    echo "Waiting for window (xdotool mode)..."
    WID=""
    for _ in $(seq 1 20); do
        WID=$(xdotool search --name "OASIS" 2>/dev/null | head -1) || true
        [[ -n "$WID" ]] && break
        sleep 0.5
    done
    if [[ -n "$WID" ]]; then
        echo "Window found. Sending test clicks..."
        sleep 2
        xdotool mousemove --window "$WID" 400 350
        xdotool click --window "$WID" 1
        sleep 1
        xdotool mousemove --window "$WID" 500 400
        xdotool click --window "$WID" 1
    fi
fi

# Wait for app to exit (auto-exit via OASIS_TV_TIMEOUT, or manual close)
echo "Waiting for test to complete (up to $((TIMEOUT + 60))s)..."
DEADLINE=$((SECONDS + TIMEOUT + 60))
while kill -0 "$APP_PID" 2>/dev/null && [[ $SECONDS -lt $DEADLINE ]]; do
    sleep 1
done

# Kill if still running
if kill -0 "$APP_PID" 2>/dev/null; then
    echo "Force-killing app (timed out)..."
    kill "$APP_PID" 2>/dev/null || true
fi
wait "$APP_PID" 2>/dev/null || true
APP_PID=""

echo ""
echo "========================================="
echo "  TV Streaming Video Decode Test Results"
echo "========================================="
echo ""

# Key log lines
echo "--- Key Events ---"
grep -E "(auto-tune|auto-exit|decode|TV:|seeked|error|audio device)" "$LOGFILE" \
    | grep -iv "stbl\|atom\|fetch_tv\|background fetch\|std_backend" \
    | head -25
echo ""

# Metrics
HAS_AUTOTUNE=$(grep -c "auto-tune" "$LOGFILE" || true)
HAS_ERROR=$(grep -c "video decode error" "$LOGFILE" || true)
HAS_DECODE_START=$(grep -c "software decode thread started" "$LOGFILE" || true)
HAS_SEEK=$(grep -c "seeked to" "$LOGFILE" || true)
HAS_EOF=$(grep -c "end of video stream" "$LOGFILE" || true)
HAS_AUDIO=$(grep -c "audio device opened" "$LOGFILE" || true)
HAS_TIMEOUT=$(grep -c "TV test: timeout reached" "$LOGFILE" || true)
CLEAN_EXIT=$(grep -c "shut down cleanly" "$LOGFILE" || true)

echo "Auto-tune triggered:    $HAS_AUTOTUNE"
echo "Decode threads started: $HAS_DECODE_START"
echo "Seeks completed:        $HAS_SEEK"
echo "H.264 decode errors:    $HAS_ERROR"
echo "Audio opened:           $HAS_AUDIO"
echo "EOF reached:            $HAS_EOF"
echo "Timeout auto-exit:      $HAS_TIMEOUT"
echo "Clean shutdown:         $CLEAN_EXIT"
echo ""

# Verdict
if [[ "$HAS_ERROR" -gt 0 ]]; then
    echo "FAIL: H.264 decode errors detected"
    grep "video decode error" "$LOGFILE"
    exit 1
elif [[ "$HAS_AUTOTUNE" -eq 0 ]]; then
    echo "FAIL: Auto-tune never triggered (catalog fetch may have failed)"
    exit 1
elif [[ "$HAS_DECODE_START" -eq 0 ]]; then
    echo "FAIL: Decode thread never started"
    exit 1
elif [[ "$HAS_TIMEOUT" -gt 0 && "$HAS_ERROR" -eq 0 ]]; then
    echo "PASS: Video decoded for ${TIMEOUT}s without errors"
    exit 0
elif [[ "$HAS_EOF" -gt 0 && "$HAS_ERROR" -eq 0 ]]; then
    echo "PASS: Video reached EOF without errors"
    exit 0
elif [[ "$CLEAN_EXIT" -gt 0 && "$HAS_ERROR" -eq 0 ]]; then
    echo "PASS: Clean exit without decode errors"
    exit 0
else
    echo "UNKNOWN: Check logs at $LOGFILE"
    exit 1
fi
