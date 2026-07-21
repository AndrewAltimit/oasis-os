#!/usr/bin/env bash
# test-settings-ppsspp.sh -- Drive the PSP Settings (theme picker) flow in PPSSPP
# via the AUTORUN.txt scaffolding.
#
# Builds the EBOOT with the `autorun-script` cargo feature, drops an
# AUTORUN.txt onto the emulated memstick, launches PPSSPP, and waits for the
# script to write `autorun.done`. Screenshots are raw 480x272 ABGR dumps
# converted to PNG host-side via Pillow.
#
# Requires: cargo+nightly with rust-psp, docker, python3 + Pillow.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
PSP_CRATE_DIR="$PROJECT_DIR/crates/oasis-backend-psp"
ROM_DIR="$PSP_CRATE_DIR/target/mipsel-sony-psp-std"
EBOOT="$ROM_DIR/release/EBOOT.PBP"
OUT_DIR="$PROJECT_DIR/screenshots/settings-test"
MEMSTICK_DIR="$OUT_DIR/memstick"
# NB: PPSSPP's MemStickRoot is its memstick dir treated as `ms0:/PSP/`,
# so `ms0:/PSP/GAME/OASISOS/<f>` maps to `<MemStickRoot>/GAME/OASISOS/<f>`
# (the leading `PSP/` is stripped). On real hardware the EBOOT writes to
# `ms0:/PSP/GAME/OASISOS/...` which has the same effective layout.
OASIS_DIR="$MEMSTICK_DIR/GAME/OASISOS"
CONTAINER_NAME="settings-test-ppsspp"
TIMEOUT_S="${TIMEOUT_S:-90}"

cleanup() {
    docker stop "$CONTAINER_NAME" 2>/dev/null || true
    docker rm "$CONTAINER_NAME" 2>/dev/null || true
}
trap cleanup EXIT

log() { echo "[$(date +%H:%M:%S)] $*"; }

log "=== PSP Settings UI Test (autorun) ==="

# -----------------------------------------------------------------------
# 1. Build EBOOT with autorun-script feature
# -----------------------------------------------------------------------
log "Building EBOOT with --features autorun-script..."
( cd "$PSP_CRATE_DIR" \
  && RUST_PSP_BUILD_STD=1 cargo psp --release --features autorun-script ) \
  > /tmp/eboot-build.log 2>&1 || {
    log "BUILD FAILED — see /tmp/eboot-build.log"
    tail -20 /tmp/eboot-build.log
    exit 1
}
[[ -f "$EBOOT" ]] || { log "EBOOT.PBP missing after build"; exit 1; }

# -----------------------------------------------------------------------
# 2. Stage the AUTORUN.txt and a clean memstick
# -----------------------------------------------------------------------
log "Staging memstick at $MEMSTICK_DIR"
rm -rf "$OUT_DIR" 2>/dev/null || sudo rm -rf "$OUT_DIR"
mkdir -p "$OASIS_DIR"
chmod -R 0777 "$MEMSTICK_DIR"

cat > "$OASIS_DIR/AUTORUN.txt" <<'EOF'
# Open Settings → pick Retro CGA → apply → screenshot at each step.
# Frame budget: PPSSPP ~60fps, so wait <N> ≈ N/60 seconds.

log boot ok, capturing dashboard
wait 60
screenshot ms0:/PSP/GAME/OASISOS/01-dashboard-initial.bmp

log opening Settings via launch
launch settings
wait 90
screenshot ms0:/PSP/GAME/OASISOS/02-settings-opened.bmp

log navigate down to Retro CGA (preset index 3)
press down
wait 6
press down
wait 6
press down
wait 30
screenshot ms0:/PSP/GAME/OASISOS/03-retrocga-highlighted.bmp

log apply theme
press cross
wait 60
screenshot ms0:/PSP/GAME/OASISOS/04-theme-applied.bmp

log close Settings via Cancel
press circle
wait 60
screenshot ms0:/PSP/GAME/OASISOS/05-dashboard-with-new-theme.bmp

log all done
exit 0
EOF
log "AUTORUN.txt:"
sed 's/^/  /' "$OASIS_DIR/AUTORUN.txt"

# -----------------------------------------------------------------------
# 3. Launch PPSSPP with memstick mounted writable
# -----------------------------------------------------------------------
log "Launching PPSSPP..."
docker run --rm -d \
    --name "$CONTAINER_NAME" \
    -e PPSSPP_HEADLESS=0 \
    -e DISPLAY="${DISPLAY:-:0}" \
    -e NVIDIA_DRIVER_CAPABILITIES=all \
    -e NVIDIA_VISIBLE_DEVICES=all \
    -v "/tmp/.X11-unix:/tmp/.X11-unix:ro" \
    -v "$HOME/.Xauthority:/home/ppsspp/.Xauthority:ro" \
    -v "$ROM_DIR:/roms:ro" \
    -v "$MEMSTICK_DIR:/home/ppsspp/.config/ppsspp/PSP" \
    --runtime=nvidia --network=host --device /dev/dri:/dev/dri \
    "oasis-os-ppsspp:latest" /roms/release/EBOOT.PBP > /dev/null

# -----------------------------------------------------------------------
# 4. Find the PPSSPP window so we can scrot it on demand.
# -----------------------------------------------------------------------
WID=""
for _ in {1..30}; do
    WID=$(xdotool search --class "PPSSPP" 2>/dev/null | head -1 || true)
    [[ -n "$WID" ]] && break
    sleep 1
done
[[ -n "$WID" ]] || { log "ERROR: PPSSPP window not found"; exit 1; }
log "PPSSPP window: $WID"

# -----------------------------------------------------------------------
# 5. Watch for screenshot request sentinels until autorun.done or timeout.
#    Each sentinel is a 0-byte file `<path>.req` written by the autorun
#    runner. We capture the PPSSPP window via scrot, save to <path>.png,
#    delete the .req. Loop exits when autorun.done appears (clean exit)
#    or container stops.
# -----------------------------------------------------------------------
log "Watching for screenshot requests (timeout ${TIMEOUT_S}s)..."
park_x11_cursor() { xdotool mousemove 1900 1000 2>/dev/null || true; }
focus_ppsspp() {
    xdotool windowactivate --sync "$WID" 2>/dev/null || true
    sleep 0.15
}

waited=0
while [[ $waited -lt $TIMEOUT_S ]]; do
    # Capture any pending screenshot requests.
    shopt -s nullglob
    for req in "$OASIS_DIR"/*.bmp.req; do
        # Strip ".req" → original ms0: path; we want just the basename
        # without any extension for the PNG name.
        bmp_path="${req%.req}"
        base=$(basename "$bmp_path" .bmp)
        out="$OUT_DIR/${base}.png"
        park_x11_cursor
        focus_ppsspp
        if scrot -u "$out" 2>/dev/null; then
            log "  captured $out"
        else
            log "  WARN scrot failed for $base"
        fi
        rm -f "$req"
    done
    shopt -u nullglob

    if [[ -f "$OASIS_DIR/autorun.done" ]]; then
        log "autorun complete (exit $(tr -d '\n' < "$OASIS_DIR/autorun.done"))"
        break
    fi
    if ! docker inspect -f '{{.State.Running}}' "$CONTAINER_NAME" 2>/dev/null | grep -q true; then
        log "PPSSPP container exited"
        break
    fi
    sleep 1
    waited=$((waited + 1))
done

# Drain any final pending sentinels (race between exit and sentinel polling).
shopt -s nullglob
for req in "$OASIS_DIR"/*.bmp.req; do
    bmp_path="${req%.req}"
    base=$(basename "$bmp_path" .bmp)
    out="$OUT_DIR/${base}.png"
    [[ -f "$out" ]] || scrot -u "$out" 2>/dev/null || true
    rm -f "$req"
done
shopt -u nullglob

# -----------------------------------------------------------------------
# 6. Surface log + done marker
# -----------------------------------------------------------------------
if [[ -f "$OASIS_DIR/autorun.log" ]]; then
    log "----- autorun.log -----"
    sed 's/^/  /' "$OASIS_DIR/autorun.log"
    log "-----------------------"
fi

log ""
log "=== Outputs (in $OUT_DIR) ==="
shopt -s nullglob
pngs=("$OUT_DIR"/*.png)
shopt -u nullglob
if (( ${#pngs[@]} == 0 )); then
    log "  (no PNGs produced)"
else
    printf '  %s\n' "${pngs[@]}"
fi
