#!/usr/bin/env bash
# psp-selftest.sh -- Run OASIS OS built-in self-test in PPSSPP headless mode.
#
# Creates the SELFTEST sentinel on a host-side memstick directory, launches
# PPSSPP headless via docker compose, then reads the selftest.log output.
#
# Exit code matches the selftest EXIT_CODE (0 = all pass, 1 = failure).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

MEMSTICK="$PROJECT_DIR/crates/oasis-backend-psp/target/psp-memstick"
GAME_DIR="$MEMSTICK/PSP/GAME/OASISOS"
SENTINEL="$MEMSTICK/SELFTEST"
LOG_FILE="$GAME_DIR/selftest.log"
TIMEOUT="${PPSSPP_TIMEOUT:-60}"

echo "=== OASIS OS PSP Self-Test ==="
echo ""

# 1. Create memstick directory structure and sentinel file.
mkdir -p "$GAME_DIR"
touch "$SENTINEL"
echo "Created sentinel: $SENTINEL"

# Remove stale log if present.
rm -f "$LOG_FILE"

# 2. Run PPSSPP headless via docker compose.
echo "Starting PPSSPP headless (timeout: ${TIMEOUT}s)..."
timeout "$TIMEOUT" \
    docker compose --profile psp run --rm \
    -e PPSSPP_HEADLESS=1 \
    ppsspp /roms/release/EBOOT.PBP \
    2>&1 || true

echo ""

# 3. Read and display selftest.log.
if [ ! -f "$LOG_FILE" ]; then
    echo "ERROR: selftest.log not found at $LOG_FILE"
    echo "PPSSPP may have crashed or timed out before the test completed."
    exit 1
fi

echo "--- selftest.log ---"
cat "$LOG_FILE"
echo "--- end ---"
echo ""

# 4. Parse EXIT_CODE line and exit accordingly.
EXIT_LINE=$(grep -o 'EXIT_CODE: [0-9]*' "$LOG_FILE" || true)
if [ -z "$EXIT_LINE" ]; then
    echo "ERROR: No EXIT_CODE found in selftest.log"
    exit 1
fi

CODE=$(echo "$EXIT_LINE" | grep -o '[0-9]*$')
if [ "$CODE" = "0" ]; then
    echo "Self-test PASSED"
else
    echo "Self-test FAILED"
fi

exit "$CODE"
