#!/usr/bin/env bash
# test-all-themes-ppsspp.sh -- Cycle through every PSP skin preset via autorun
# and screenshot the dashboard for each. Use the `skin` autorun verb directly
# (no UI navigation) so we end up with a clean side-by-side comparison.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
PSP_CRATE_DIR="$PROJECT_DIR/crates/oasis-backend-psp"
ROM_DIR="$PSP_CRATE_DIR/target/mipsel-sony-psp-std"
OUT_DIR="$PROJECT_DIR/screenshots/all-themes"
MEMSTICK_DIR="$OUT_DIR/memstick"
OASIS_DIR="$MEMSTICK_DIR/GAME/OASISOS"
CONTAINER_NAME="all-themes-ppsspp"
TIMEOUT_S="${TIMEOUT_S:-180}"

cleanup() {
    docker stop "$CONTAINER_NAME" 2>/dev/null || true
    docker rm "$CONTAINER_NAME" 2>/dev/null || true
}
trap cleanup EXIT

log() { echo "[$(date +%H:%M:%S)] $*"; }

THEMES=(psix classic balatro retro-cga solarized highcontrast altimit)

log "=== PSP All-Theme Dashboard Test ==="

# Build EBOOT with autorun-script feature.
log "Building EBOOT..."
( cd "$PSP_CRATE_DIR" \
  && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release --features autorun-script ) \
  > /tmp/eboot-build.log 2>&1 || {
    log "BUILD FAILED — see /tmp/eboot-build.log"
    tail -20 /tmp/eboot-build.log
    exit 1
}

# Stage memstick.
rm -rf "$OUT_DIR" 2>/dev/null || sudo rm -rf "$OUT_DIR"
mkdir -p "$OASIS_DIR"
chmod -R 0777 "$MEMSTICK_DIR"

# Build AUTORUN.txt — for each theme: skin <key>; wait; screenshot.
{
    echo "log boot ok, starting theme sweep"
    echo "wait 60"
    for theme in "${THEMES[@]}"; do
        echo ""
        echo "log === switching to $theme ==="
        echo "skin $theme"
        echo "wait 30"
        echo "screenshot ms0:/PSP/GAME/OASISOS/${theme}.bmp"
    done
    echo ""
    echo "log all themes captured"
    echo "exit 0"
} > "$OASIS_DIR/AUTORUN.txt"
log "AUTORUN.txt written ($(wc -l < "$OASIS_DIR/AUTORUN.txt") lines)"

# Launch PPSSPP.
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

# Find the window for scrot.
WID=""
for _ in {1..30}; do
    WID=$(xdotool search --class "PPSSPP" 2>/dev/null | head -1 || true)
    [[ -n "$WID" ]] && break
    sleep 1
done
[[ -n "$WID" ]] || { log "ERROR: PPSSPP window not found"; exit 1; }
log "PPSSPP window: $WID"

# Watch for screenshot sentinels.
log "Watching for screenshot requests (timeout ${TIMEOUT_S}s)..."
waited=0
while [[ $waited -lt $TIMEOUT_S ]]; do
    shopt -s nullglob
    for req in "$OASIS_DIR"/*.bmp.req; do
        bmp_path="${req%.req}"
        base=$(basename "$bmp_path" .bmp)
        out="$OUT_DIR/${base}.png"
        xdotool mousemove 1900 1000 2>/dev/null || true
        xdotool windowactivate --sync "$WID" 2>/dev/null || true
        sleep 0.3
        if scrot -u "$out" 2>/dev/null; then
            log "  captured $out"
        else
            log "  WARN scrot failed for $base"
        fi
        rm -f "$req"
    done
    shopt -u nullglob

    [[ -f "$OASIS_DIR/autorun.done" ]] && { log "autorun done"; break; }
    docker inspect -f '{{.State.Running}}' "$CONTAINER_NAME" 2>/dev/null | grep -q true \
        || { log "container exited"; break; }
    sleep 0.25
    waited=$((waited + 1))
done

# Summary.
log ""
log "=== Captured themes ==="
shopt -s nullglob
pngs=("$OUT_DIR"/*.png)
shopt -u nullglob
if (( ${#pngs[@]} == 0 )); then
    log "  (none)"
else
    printf '  %s\n' "${pngs[@]}"
fi

if [[ -f "$OASIS_DIR/autorun.log" ]]; then
    log ""
    log "----- autorun.log (last 20 lines) -----"
    tail -20 "$OASIS_DIR/autorun.log" | sed 's/^/  /'
fi
