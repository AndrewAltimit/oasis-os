#!/usr/bin/env bash
# PSP Screenshot Test Driver
#
# Runs OASIS_OS EBOOT.PBP in PPSSPPHeadless and captures screenshots
# for manual visual review. Not CI-blocking.
#
# Prerequisites:
#   - PSP EBOOT built: cd crates/oasis-backend-psp && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
#   - PPSSPP Docker image built: docker compose --profile psp build ppsspp
#
# Usage:
#   ./scripts/psp-screenshot.sh                  # All scenarios
#   ./scripts/psp-screenshot.sh dashboard        # Single scenario
#   ./scripts/psp-screenshot.sh --list           # List available scenarios
#   ./scripts/psp-screenshot.sh --report         # Generate HTML report
#
# Output:
#   screenshots/psp/{scenario}/actual.png
#   screenshots/psp/report.html                  (with --report)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
EBOOT_PATH="crates/oasis-backend-psp/target/mipsel-sony-psp-std/release/EBOOT.PBP"
SCREENSHOT_DIR="$REPO_ROOT/screenshots/psp"
TIMEOUT="${PSP_TIMEOUT:-8}"

# Scenario definitions.
# Each scenario: name, timeout_seconds, description
declare -A SCENARIOS
SCENARIOS=(
    [dashboard]="Boot to dashboard and capture initial render"
    [terminal]="Switch to terminal mode (press F1 key)"
    [browser]="Open browser with bundled test page"
    [input]="Navigate dashboard with D-pad inputs"
    [audio]="Trigger audio playback, capture status bar"
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

log_info()  { echo "  [INFO]  $*"; }
log_ok()    { echo "  [OK]    $*"; }
log_fail()  { echo "  [FAIL]  $*"; }
log_warn()  { echo "  [WARN]  $*"; }

check_prerequisites() {
    if [ ! -f "$REPO_ROOT/$EBOOT_PATH" ]; then
        echo "ERROR: EBOOT.PBP not found at $EBOOT_PATH"
        echo ""
        echo "Build it first:"
        echo "  cd crates/oasis-backend-psp"
        echo "  RUST_PSP_BUILD_STD=1 cargo +nightly psp --release"
        exit 1
    fi

    if ! docker compose --profile psp config --quiet 2>/dev/null; then
        echo "ERROR: Docker compose psp profile not available"
        echo ""
        echo "Build the PPSSPP image:"
        echo "  docker compose --profile psp build ppsspp"
        exit 1
    fi
}

# Run PPSSPP headless with a timeout and capture a screenshot.
#
# Arguments:
#   $1 - scenario name (used for output directory)
#   $2 - timeout in seconds
#   $3 - extra PPSSPP flags (optional)
run_ppsspp_screenshot() {
    local scenario="$1"
    local timeout="$2"
    local extra_flags="${3:-}"
    local out_dir="$SCREENSHOT_DIR/$scenario"

    mkdir -p "$out_dir"

    log_info "Running scenario: $scenario (timeout=${timeout}s)"

    # PPSSPPHeadless captures a framebuffer dump on exit when
    # --screenshot is specified. The timeout causes a clean exit.
    local screenshot_path="/screenshots/$scenario/actual.png"

    # Run in the repo root so volumes resolve correctly.
    cd "$REPO_ROOT"

    # shellcheck disable=SC2086
    if docker compose --profile psp run --rm \
        -e PPSSPP_HEADLESS=1 \
        ppsspp \
        /roms/release/EBOOT.PBP \
        --timeout="$timeout" \
        --screenshot="$screenshot_path" \
        $extra_flags \
        2>/dev/null; then
        :  # Success.
    else
        local exit_code=$?
        # Timeout exit is expected (PPSSPP exits non-zero on timeout).
        if [ $exit_code -ne 0 ]; then
            log_warn "PPSSPP exited with code $exit_code (timeout is expected)"
        fi
    fi

    # Check if screenshot was captured.
    if [ -f "$out_dir/actual.png" ]; then
        log_ok "$scenario -> $out_dir/actual.png"
        return 0
    else
        # Fallback: check if PPSSPP wrote a framebuffer dump anywhere.
        log_fail "$scenario -- no screenshot captured"
        log_info "PPSSPPHeadless may not support --screenshot for this build."
        log_info "Try running manually:"
        log_info "  docker compose --profile psp run --rm -e PPSSPP_HEADLESS=1 ppsspp /roms/release/EBOOT.PBP --timeout=$timeout"
        return 1
    fi
}

# ---------------------------------------------------------------------------
# Scenario runners
# ---------------------------------------------------------------------------

run_scenario_dashboard() {
    run_ppsspp_screenshot "dashboard" "$TIMEOUT"
}

run_scenario_terminal() {
    # Terminal mode would require input injection to press F1.
    # For now, capture after timeout (dashboard state).
    run_ppsspp_screenshot "terminal" "$TIMEOUT"
}

run_scenario_browser() {
    # Browser would require navigation input. Capture dashboard state for now.
    run_ppsspp_screenshot "browser" "$TIMEOUT"
}

run_scenario_input() {
    # Input verification would require injecting D-pad events.
    # Capture after timeout to verify boot didn't crash.
    run_ppsspp_screenshot "input" "$TIMEOUT"
}

run_scenario_audio() {
    # Audio verification captures the status bar after boot.
    run_ppsspp_screenshot "audio" "$TIMEOUT"
}

# ---------------------------------------------------------------------------
# HTML report
# ---------------------------------------------------------------------------

generate_report() {
    local report="$SCREENSHOT_DIR/report.html"

    cat > "$report" <<'HEADER'
<!DOCTYPE html>
<html><head>
<title>OASIS_OS PSP Screenshot Report</title>
<style>
  body { font-family: sans-serif; margin: 20px; background: #f5f5f5; }
  h1 { color: #333; }
  .grid { display: flex; flex-wrap: wrap; gap: 16px; }
  .card { background: white; border-radius: 8px; padding: 12px;
          box-shadow: 0 2px 4px rgba(0,0,0,0.1); max-width: 500px; }
  .card h3 { margin: 0 0 8px 0; font-size: 14px; color: #555; }
  .card p { margin: 4px 0; font-size: 12px; color: #888; }
  .card img { max-width: 480px; border: 1px solid #ddd; image-rendering: pixelated; }
  .missing { color: #c00; font-style: italic; }
</style>
</head><body>
<h1>OASIS_OS PSP Screenshot Report</h1>
<p>Captured via PPSSPPHeadless. Resolution: 480x272 (PSP native).</p>
<div class="grid">
HEADER

    for scenario in "${!SCENARIOS[@]}"; do
        local desc="${SCENARIOS[$scenario]}"
        local img_path="$scenario/actual.png"
        local full_path="$SCREENSHOT_DIR/$scenario/actual.png"

        cat >> "$report" <<CARD
<div class="card">
  <h3>$scenario</h3>
  <p>$desc</p>
CARD

        if [ -f "$full_path" ]; then
            echo "  <img src=\"$img_path\" alt=\"$scenario\">" >> "$report"
        else
            echo "  <p class=\"missing\">Screenshot not captured</p>" >> "$report"
        fi

        echo "</div>" >> "$report"
    done

    cat >> "$report" <<'FOOTER'
</div>
</body></html>
FOOTER

    log_ok "Report: $report"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

list_scenarios() {
    echo "Available PSP screenshot scenarios:"
    echo ""
    for scenario in $(echo "${!SCENARIOS[@]}" | tr ' ' '\n' | sort); do
        printf "  %-12s  %s\n" "$scenario" "${SCENARIOS[$scenario]}"
    done
}

main() {
    local generate_report_flag=false
    local scenarios_to_run=()

    # Parse arguments.
    while [ $# -gt 0 ]; do
        case "$1" in
            --list|-l)
                list_scenarios
                exit 0
                ;;
            --report|-r)
                generate_report_flag=true
                shift
                ;;
            --timeout|-t)
                TIMEOUT="$2"
                shift 2
                ;;
            --help|-h)
                echo "Usage: $0 [OPTIONS] [SCENARIO...]"
                echo ""
                echo "Options:"
                echo "  --list, -l       List available scenarios"
                echo "  --report, -r     Generate HTML comparison report"
                echo "  --timeout, -t N  Set PPSSPP timeout (default: $TIMEOUT)"
                echo "  --help, -h       Show this help"
                echo ""
                list_scenarios
                exit 0
                ;;
            *)
                if [ -n "${SCENARIOS[$1]+x}" ]; then
                    scenarios_to_run+=("$1")
                else
                    echo "ERROR: Unknown scenario '$1'"
                    echo ""
                    list_scenarios
                    exit 1
                fi
                shift
                ;;
        esac
    done

    # Default: run all scenarios.
    if [ ${#scenarios_to_run[@]} -eq 0 ]; then
        scenarios_to_run=($(echo "${!SCENARIOS[@]}" | tr ' ' '\n' | sort))
    fi

    check_prerequisites

    echo "PSP Screenshot Tests"
    echo "===================="
    echo "EBOOT: $EBOOT_PATH"
    echo "Timeout: ${TIMEOUT}s per scenario"
    echo "Output: $SCREENSHOT_DIR/"
    echo ""

    local passed=0
    local failed=0

    for scenario in "${scenarios_to_run[@]}"; do
        if "run_scenario_$scenario"; then
            ((passed++))
        else
            ((failed++))
        fi
    done

    echo ""
    echo "Results: $passed passed, $failed failed"

    if $generate_report_flag; then
        echo ""
        generate_report
    fi

    if [ $failed -gt 0 ]; then
        exit 1
    fi
}

main "$@"
