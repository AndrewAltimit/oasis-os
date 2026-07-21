#!/usr/bin/env bash
# test-osk-ppsspp.sh -- Automated OSK crash test in PPSSPP GUI mode.
#
# Launches PPSSPP SDL with the PSP EBOOT, navigates to Terminal view,
# triggers the OSK via Square button, and captures screenshots at each
# step to verify no crash occurs.
#
# Requirements: xdotool, scrot, docker
# Usage: ./scripts/test-osk-ppsspp.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SCREENSHOT_DIR="$PROJECT_DIR/screenshots/osk-test"
ROM_DIR="$PROJECT_DIR/crates/oasis-backend-psp/target/mipsel-sony-psp-std"
EBOOT="$ROM_DIR/release/EBOOT.PBP"

# PPSSPP SDL default QWERTY keyboard mappings (from PPSSPP source:
# Core/KeyMapDefaults.cpp -> defaultQwertyKeyboardKeyMap).
# Linux uses Enter for Select (Windows uses V).
#
# This is the complete button map on purpose, so a new scenario can reach for
# any button without re-deriving it; only the subset a scenario drives is read.
# One `declare` so the SC2034 waiver covers the whole map.
# shellcheck disable=SC2034
declare \
    KEY_CROSS="z" \
    KEY_CIRCLE="x" \
    KEY_SQUARE="a" \
    KEY_TRIANGLE="s" \
    KEY_START="space" \
    KEY_SELECT="Return" \
    KEY_L="q" \
    KEY_R="w" \
    KEY_UP="Up" \
    KEY_DOWN="Down" \
    KEY_LEFT="Left" \
    KEY_RIGHT="Right"

PPSSPP_WID=""
CONTAINER_NAME="osk-test-ppsspp"

cleanup() {
    echo "[cleanup] Stopping PPSSPP container..."
    docker stop "$CONTAINER_NAME" 2>/dev/null || true
    docker rm "$CONTAINER_NAME" 2>/dev/null || true
}
trap cleanup EXIT

log() { echo "[$(date +%H:%M:%S)] $*"; }

screenshot() {
    local name="$1"
    local path="$SCREENSHOT_DIR/${name}.png"
    if [[ -n "$PPSSPP_WID" ]]; then
        # Capture by window ID using xwd (works even if window is behind
        # other windows), then convert to PNG via Python Pillow.
        local xwd_file="${path%.png}.xwd"
        xwd -id "$PPSSPP_WID" -silent -out "$xwd_file" 2>/dev/null
        if [[ -f "$xwd_file" ]]; then
            python3 -c "
from PIL import Image
img = Image.open('$xwd_file')
img.save('$path')
" 2>/dev/null && rm -f "$xwd_file"
        fi
        # Fallback to scrot if xwd/PIL failed.
        if [[ ! -f "$path" ]]; then
            rm -f "$xwd_file"
            scrot "$path" 2>/dev/null || log "WARN: Could not capture screenshot $name"
        fi
    else
        scrot "$path" 2>/dev/null || log "WARN: Could not capture screenshot $name"
    fi
    if [[ -f "$path" ]]; then
        log "Screenshot: $path"
    fi
}

focus_ppsspp() {
    # Aggressively bring PPSSPP to front and ensure it has real X11
    # input focus.  SDL only processes key events from the focused
    # window -- the xdotool --window flag bypasses the WM and SDL
    # ignores those synthetic events.
    xdotool windowactivate --sync "$PPSSPP_WID" 2>/dev/null || true
    xdotool windowraise "$PPSSPP_WID" 2>/dev/null || true
    xdotool windowfocus --sync "$PPSSPP_WID" 2>/dev/null || true
    # Give the WM time to actually move focus.
    sleep 0.5
}

send_key() {
    local key="$1"
    local delay="${2:-0.8}"
    if [[ -z "$PPSSPP_WID" ]]; then
        log "ERROR: PPSSPP window ID not set"
        return 1
    fi
    focus_ppsspp
    # Use keydown + sleep + keyup so PPSSPP's per-frame input poll
    # (60 fps = ~16ms) has time to see the keypress.  Plain
    # `xdotool key` fires down+up instantly and gets missed.
    xdotool keydown "$key"
    sleep 0.15
    xdotool keyup "$key"
    log "Sent key: $key"
    sleep "$delay"
}

send_text() {
    local text="$1"
    if [[ -z "$PPSSPP_WID" ]]; then
        log "ERROR: PPSSPP window ID not set"
        return 1
    fi
    focus_ppsspp
    xdotool type --delay 100 "$text"
    log "Typed text: $text"
}

wait_for_window() {
    local max_wait=30
    local waited=0
    log "Waiting for PPSSPP window..."
    while [[ $waited -lt $max_wait ]]; do
        # PPSSPP SDL window has class "PPSSPP". Search by class to avoid
        # matching terminal windows whose title happens to contain "PPSSPP".
        local wid
        wid=$(xdotool search --class "PPSSPP" 2>/dev/null | head -1 || true)
        if [[ -z "$wid" ]]; then
            # Fallback: PPSSPP sets the window title to the game name
            # once loaded (e.g. "oasis-backend-psp").
            wid=$(xdotool search --name "oasis-backend-psp" 2>/dev/null | head -1 || true)
        fi
        if [[ -n "$wid" ]]; then
            PPSSPP_WID="$wid"
            log "PPSSPP window found: $PPSSPP_WID ($(xdotool getwindowname "$PPSSPP_WID" 2>/dev/null || echo '?'))"
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    log "ERROR: PPSSPP window not found after ${max_wait}s"
    # Debug: list all windows for troubleshooting.
    log "All windows:"
    xdotool search --name "" 2>/dev/null | while read -r w; do
        log "  $w: $(xdotool getwindowname "$w" 2>/dev/null || echo '?')"
    done
    return 1
}

ppsspp_alive() {
    docker inspect -f '{{.State.Running}}' "$CONTAINER_NAME" 2>/dev/null | grep -q "true"
}

# -----------------------------------------------------------------------
# Main
# -----------------------------------------------------------------------

if [[ ! -f "$EBOOT" ]]; then
    log "ERROR: EBOOT not found at $EBOOT"
    log "Build with: cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo psp --release"
    exit 1
fi

mkdir -p "$SCREENSHOT_DIR"

log "=== PPSSPP OSK Crash Test ==="
log "EBOOT: $EBOOT"
log "Screenshots: $SCREENSHOT_DIR"

# Start PPSSPP SDL (GUI mode) in a docker container.
log "Starting PPSSPP SDL..."
docker run --rm -d \
    --name "$CONTAINER_NAME" \
    -e PPSSPP_HEADLESS=0 \
    -e DISPLAY="${DISPLAY:-:0}" \
    -e NVIDIA_DRIVER_CAPABILITIES=all \
    -e NVIDIA_VISIBLE_DEVICES=all \
    -v "/tmp/.X11-unix:/tmp/.X11-unix:ro" \
    -v "$HOME/.Xauthority:/home/ppsspp/.Xauthority:ro" \
    -v "$ROM_DIR:/roms:ro" \
    --runtime=nvidia \
    --network=host \
    --device /dev/dri:/dev/dri \
    "oasis-os-ppsspp:latest" \
    /roms/release/EBOOT.PBP 2>&1 || {
        log "ERROR: Failed to start PPSSPP container"
        exit 1
    }

log "Container started: $CONTAINER_NAME"

# Wait for the PPSSPP window to appear.
wait_for_window || exit 1

# Let the app fully initialize (GU setup, skin loading, etc.).
log "Waiting for OASIS_OS to boot (8s)..."
sleep 8

if ! ppsspp_alive; then
    log "FAIL: PPSSPP crashed during boot!"
    exit 1
fi

# Step 1: Screenshot the Dashboard (initial view).
screenshot "01-dashboard"

# Step 2: Navigate to Terminal view.
# The Dashboard grid is 2 columns:
#   0: File Manager    1: Settings
#   2: Network         3: Terminal
# Selection starts at index 0 (File Manager).
# Navigate step by step with verification screenshots.
log "--- Step: Navigate to Terminal icon ---"

# D-pad Down: File Manager (0) → Network (2).
send_key "$KEY_DOWN" 1.5
screenshot "02a-after-down"

# D-pad Right: Network (2) → Terminal (3).
send_key "$KEY_RIGHT" 1.5
screenshot "02b-after-right"

# Cross: open Terminal app.
send_key "$KEY_CROSS" 2
screenshot "02-terminal"

if ! ppsspp_alive; then
    log "FAIL: PPSSPP crashed after switching to Terminal!"
    exit 1
fi

# Step 3: Press Square to open the OSK.
log "--- Step: Open OSK (Square) ---"
send_key "$KEY_SQUARE" 3
screenshot "03-osk-opened"

if ! ppsspp_alive; then
    log "FAIL: PPSSPP crashed when opening the OSK!"
    exit 1
fi

# Step 4: The PPSSPP OSK dialog should be visible now.
# Try typing some text and confirming.
log "--- Step: Type text in OSK ---"
# PPSSPP's built-in OSK dialog accepts keyboard text directly.
# Type "hello" and press Enter to confirm.
sleep 1
send_text "hello"
sleep 1
screenshot "04-osk-typed"

# Press Enter to confirm the OSK dialog (if PPSSPP shows one).
send_key "Return" 2
screenshot "05-osk-confirmed"

if ! ppsspp_alive; then
    log "FAIL: PPSSPP crashed after OSK confirmation!"
    exit 1
fi

# Step 5: Verify we're back in Terminal view.
log "--- Step: Verify Terminal still works ---"
send_key "$KEY_UP" 2   # Up in terminal = execute "help"
screenshot "06-terminal-after-osk"

if ! ppsspp_alive; then
    log "FAIL: PPSSPP crashed after returning from OSK!"
    exit 1
fi

# Step 6: Open OSK again to verify repeated use works.
log "--- Step: Open OSK a second time ---"
send_key "$KEY_SQUARE" 3
screenshot "07-osk-second"

if ! ppsspp_alive; then
    log "FAIL: PPSSPP crashed on second OSK open!"
    exit 1
fi

# Dismiss the OSK with Circle (back/cancel).
send_key "$KEY_CIRCLE" 2
screenshot "08-final"

if ! ppsspp_alive; then
    log "FAIL: PPSSPP crashed after second OSK dismiss!"
    exit 1
fi

log ""
log "=== ALL TESTS PASSED ==="
log "PPSSPP did not crash during any OSK operation."
log "Screenshots saved to: $SCREENSHOT_DIR/"
log ""
log "Review the screenshots to confirm:"
log "  01-dashboard.png       - OASIS_OS Dashboard (File Manager selected)"
log "  02a-after-down.png     - After D-pad Down (Network selected)"
log "  02b-after-right.png    - After D-pad Right (Terminal selected)"
log "  02-terminal.png        - Terminal view (after Cross)"
log "  03-osk-opened.png      - OSK dialog"
log "  04-osk-typed.png       - Text entered in OSK"
log "  05-osk-confirmed.png   - After confirming OSK"
log "  06-terminal-after-osk.png - Terminal after OSK"
log "  07-osk-second.png      - Second OSK invocation"
log "  08-final.png           - Final state"
