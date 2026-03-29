#!/usr/bin/env bash
# psp-devloop.sh — Automated PSP build-deploy-test cycle.
#
# Usage:
#   ./scripts/psp-devloop.sh deploy <eboot_path> [timeout_secs]
#   ./scripts/psp-devloop.sh screenshot
#   ./scripts/psp-devloop.sh reboot
#   ./scripts/psp-devloop.sh wait    # Wait for PSP USB to appear
#   ./scripts/psp-devloop.sh logs    # Read devloop + eboot logs
#   ./scripts/psp-devloop.sh status  # Read devloop status
#
# Requires: PSP with oasis.prx plugin and devloop=true in oasis.ini.

set -euo pipefail

# Auto-detect PSP mount point.
find_psp_mount() {
    for dir in /media/$USER/disk /media/$USER/PSP /run/media/$USER/disk; do
        if [ -d "$dir/PSP/GAME" ]; then
            echo "$dir"
            return 0
        fi
    done
    return 1
}

PSP_MOUNT=""
GAME_DIR=""
CMD_FILE=""
STATUS_FILE=""
LOG_FILE=""
EBOOT_LOG=""
init_paths() {
    PSP_MOUNT=$(find_psp_mount) || { echo "PSP not mounted"; return 1; }
    GAME_DIR="$PSP_MOUNT/PSP/GAME/OASISOS"
    CMD_FILE="$PSP_MOUNT/seplugins/.devloop_cmd"
    STATUS_FILE="$PSP_MOUNT/seplugins/.devloop_status"
    LOG_FILE="$PSP_MOUNT/seplugins/.devloop_log"
    EBOOT_LOG="$GAME_DIR/eboot.log"
}

wait_for_mount() {
    echo "Waiting for PSP USB..."
    while ! find_psp_mount >/dev/null 2>&1; do
        sleep 0.5
    done
    init_paths
    echo "PSP mounted at $PSP_MOUNT"
}

wait_for_unmount() {
    echo "Waiting for PSP to disconnect USB..."
    while find_psp_mount >/dev/null 2>&1; do
        sleep 0.5
    done
    echo "PSP USB disconnected"
}

cmd_deploy() {
    local eboot="${1:?Usage: deploy <eboot_path> [timeout]}"
    local timeout="${2:-60}"

    init_paths || { echo "PSP not mounted. Run: psp-devloop.sh wait"; exit 1; }

    echo "Deploying $eboot..."
    cp "$eboot" "$GAME_DIR/EBOOT.PBP"

    # Write command file.
    cat > "$CMD_FILE" <<EOF
cmd = launch
path = ms0:/PSP/GAME/OASISOS/EBOOT.PBP
timeout = $timeout
wifi = true
EOF

    sync
    echo "Command written. Ejecting USB..."

    # Unmount the filesystem only — do NOT power-off the USB device
    # as that shuts down the PSP (cuts USB bus power).
    local dev
    dev=$(findmnt -n -o SOURCE "$PSP_MOUNT" 2>/dev/null | head -1)
    if [ -n "$dev" ]; then
        udisksctl unmount -b "$dev" --no-user-interaction 2>/dev/null || true
    fi

    echo "Ejected. PSP should launch EBOOT in ~5 seconds."
    echo "Run: psp-devloop.sh wait  — to wait for PSP to reconnect after test."
}

cmd_screenshot() {
    init_paths || { echo "PSP not mounted"; exit 1; }
    cat > "$CMD_FILE" <<EOF
cmd = screenshot
EOF
    sync
    echo "Screenshot command written. Eject USB to trigger."
}

cmd_reboot() {
    init_paths || { echo "PSP not mounted"; exit 1; }
    cat > "$CMD_FILE" <<EOF
cmd = reboot
EOF
    sync
    echo "Reboot command written. Eject USB to trigger."
}

cmd_logs() {
    init_paths || { echo "PSP not mounted"; exit 1; }
    echo "=== Devloop Log ==="
    cat "$LOG_FILE" 2>/dev/null || echo "(empty)"
    echo ""
    echo "=== EBOOT Log ==="
    cat "$EBOOT_LOG" 2>/dev/null || echo "(empty)"
}

cmd_status() {
    init_paths || { echo "PSP not mounted"; exit 1; }
    echo "=== Devloop Status ==="
    cat "$STATUS_FILE" 2>/dev/null || echo "(no status)"
}

PSP_IP="${PSP_IP:-192.168.0.249}"
PSP_PORT="${PSP_PORT:-9293}"

cmd_tcp_deploy() {
    local eboot="${1:?Usage: tcp-deploy <eboot_path>}"
    local size
    size=$(stat -c%s "$eboot")
    echo "Deploying $eboot ($size bytes) to PSP at $PSP_IP..."
    (echo "deploy $size"; cat "$eboot") | nc -w 30 "$PSP_IP" "$PSP_PORT"
    echo "Deploy complete."
}

cmd_tcp_reboot() {
    echo "Sending cold reboot..."
    echo "reboot" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_ping() {
    echo "ping" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_log() {
    echo "log" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_logfull() {
    echo "logfull" | nc -w 5 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_status() {
    echo "status" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_press() {
    local btn="${1:?Usage: press <button>}"
    echo "press $btn" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_hold() {
    local btn="${1:?Usage: hold <button> <ms>}"
    local ms="${2:?Usage: hold <button> <ms>}"
    echo "hold $btn $ms" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_cursor() {
    local x="${1:?Usage: cursor <x> <y>}"
    local y="${2:?Usage: cursor <x> <y>}"
    echo "cursor $x $y" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_screenshot() {
    echo "screenshot" | nc -w 3 "$PSP_IP" "$PSP_PORT"
}

cmd_tcp_screencap() {
    local out="${1:-/tmp/psp_screen.png}"
    local raw="/tmp/psp_screen.raw"
    echo "Capturing screen from PSP..."
    echo "screencap" | nc -w 5 "$PSP_IP" "$PSP_PORT" > "$raw"
    # Strip header line "480 272\n", convert ABGR raw to PNG.
    local header_len
    header_len=$(head -1 "$raw" | wc -c)
    tail -c +$((header_len + 1)) "$raw" | \
        ffmpeg -y -f rawvideo -pixel_format abgr -video_size 480x272 \
        -i - "$out" 2>/dev/null && echo "Saved to $out" || \
        echo "Raw saved to $raw (install ffmpeg for PNG conversion)"
}

# Full cycle: deploy new EBOOT over WiFi and reboot.
cmd_tcp_cycle() {
    local eboot="${1:?Usage: cycle <eboot_path>}"
    cmd_tcp_deploy "$eboot"
    echo "Rebooting PSP..."
    cmd_tcp_reboot
    echo "Waiting 40s for restart + WiFi..."
    sleep 40
    cmd_tcp_ping
}

case "${1:-help}" in
    deploy)      cmd_deploy "${2:-}" "${3:-60}" ;;
    screenshot)  cmd_screenshot ;;
    reboot)      cmd_reboot ;;
    wait)        wait_for_mount ;;
    logs)        cmd_logs ;;
    status)      cmd_status ;;
    tcp-deploy)  cmd_tcp_deploy "${2:-}" ;;
    tcp-reboot)  cmd_tcp_reboot ;;
    tcp-ping)    cmd_tcp_ping ;;
    tcp-log)     cmd_tcp_log ;;
    tcp-logfull) cmd_tcp_logfull ;;
    tcp-status)  cmd_tcp_status ;;
    tcp-press)   cmd_tcp_press "${2:-}" ;;
    tcp-hold)    cmd_tcp_hold "${2:-}" "${3:-}" ;;
    tcp-cursor)  cmd_tcp_cursor "${2:-}" "${3:-}" ;;
    tcp-screenshot) cmd_tcp_screenshot ;;
    tcp-screencap) cmd_tcp_screencap "${2:-/tmp/psp_screen.png}" ;;
    cycle)       cmd_tcp_cycle "${2:-}" ;;
    *)
        echo "Usage: psp-devloop.sh <command> [args]"
        echo ""
        echo "WiFi commands (PSP_IP=$PSP_IP):"
        echo "  tcp-ping                  Test TCP connection"
        echo "  tcp-status                Get app state + memory info (JSON)"
        echo "  tcp-log                   Read last 2KB of EBOOT log"
        echo "  tcp-logfull               Read last 8KB of EBOOT log"
        echo "  tcp-screenshot            Take VRAM screenshot"
        echo "  tcp-press <button>        Send button press (cross,circle,up,"
        echo "                            down,left,right,triangle,square,"
        echo "                            start,select,l,r)"
        echo "  tcp-hold <button> <ms>    Hold button for N milliseconds"
        echo "  tcp-cursor <x> <y>        Move cursor to position"
        echo "  tcp-deploy <eboot>        Deploy EBOOT over WiFi"
        echo "  tcp-reboot                Cold reboot PSP"
        echo "  cycle <eboot>             Deploy + reboot + wait + ping"
        echo ""
        echo "USB commands:"
        echo "  deploy <eboot> [timeout]  Copy EBOOT via USB"
        echo "  wait                      Wait for PSP USB to appear"
        echo "  logs                      Read logs from USB"
        ;;
esac
