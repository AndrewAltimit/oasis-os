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

case "${1:-help}" in
    deploy)     cmd_deploy "${2:-}" "${3:-60}" ;;
    screenshot) cmd_screenshot ;;
    reboot)     cmd_reboot ;;
    wait)       wait_for_mount ;;
    logs)       cmd_logs ;;
    status)     cmd_status ;;
    *)
        echo "Usage: psp-devloop.sh {deploy|screenshot|reboot|wait|logs|status}"
        echo ""
        echo "  deploy <eboot> [timeout]  Copy EBOOT and launch on PSP"
        echo "  screenshot                Request framebuffer screenshot"
        echo "  reboot                    Force PSP reboot"
        echo "  wait                      Wait for PSP USB to appear"
        echo "  logs                      Read devloop + EBOOT logs"
        echo "  status                    Read devloop status"
        ;;
esac
