#!/usr/bin/env bash
# psp-smoke-test.sh -- boot the PSP EBOOTs in PPSSPPHeadless and fail unless
# they actually come up.
#
# Checks (each one can fail):
#   backend boot   OASIS-OS EBOOT runs until the timeout (an early exit means a
#                  crash or a quit), eboot.log on ms0: shows psp_main, the main
#                  loop and at least 300 rendered frames, and PPSSPP reports no
#                  bad memory/execution access.
#   backend self   With the ms0:/SELFTEST sentinel the EBOOT runs its built-in
#                  self-test, writes selftest.log with `EXIT_CODE: 0` and exits
#                  on its own before the timeout.
#   recovery boot  Recovery EBOOT runs until the timeout, reaches its TCP server
#                  (sceNetInetListen) and PPSSPP reports no bad access.
#
# Usage:
#   scripts/psp-smoke-test.sh [--backend EBOOT.PBP] [--recovery EBOOT.PBP]
#
#   With no arguments both release EBOOTs are tested at their cargo-psp
#   output paths. Pass a single flag to test only that EBOOT.
#
# Environment:
#   PPSSPP_IMAGE        Docker image whose PPSSPPHeadless binary is used. When
#                       unset, docker/ppsspp.Dockerfile is built (layer-cached)
#                       and tagged oasis-ppsspp-headless:local.
#   PSP_SMOKE_TIMEOUT   Seconds per boot run (default 20).
#   PSP_SMOKE_WORKDIR   Scratch directory (default: a fresh mktemp dir). Kept
#                       after the run so the logs can be inspected.

set -euo pipefail

# Absolute path of a directory, as Docker can mount it. Under Git Bash on
# Windows `pwd -W` gives the C:/... form Docker Desktop needs; elsewhere it
# fails and plain `pwd` is used.
abs_dir() { (cd "$1" && { pwd -W 2>/dev/null || pwd; }); }

PROJECT_DIR="$(abs_dir "$(dirname "$0")/..")"

TIMEOUT="${PSP_SMOKE_TIMEOUT:-20}"
SELFTEST_TIMEOUT=60
MIN_FRAME=300
# PPSSPP -l lines that mean the guest crashed (halted on a bad jump) or
# touched unmapped memory (PPSSPP ignores those by default and keeps going).
CRASH_RE='Bad execution access|Invalid exec address|Bad memory access'

BACKEND=""
RECOVERY=""
if [ "$#" -eq 0 ]; then
    BACKEND="$PROJECT_DIR/crates/oasis-backend-psp/target/mipsel-sony-psp-std/release/EBOOT.PBP"
    RECOVERY="$PROJECT_DIR/crates/oasis-recovery-psp/target/mipsel-sony-psp-std/release/EBOOT.PBP"
fi
while [ "$#" -gt 0 ]; do
    case "$1" in
        --backend) BACKEND="${2:?--backend needs a path}"; shift 2 ;;
        --recovery) RECOVERY="${2:?--recovery needs a path}"; shift 2 ;;
        -h | --help) sed -n '2,/^$/{/^#/p}' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

WORKDIR="${PSP_SMOKE_WORKDIR:-$(mktemp -d "${TMPDIR:-/tmp}/psp-smoke.XXXXXX")}"
mkdir -p "$WORKDIR"
WORKDIR="$(abs_dir "$WORKDIR")"

if [ -z "${PPSSPP_IMAGE:-}" ]; then
    PPSSPP_IMAGE="oasis-ppsspp-headless:local"
    echo "Building $PPSSPP_IMAGE from docker/ppsspp.Dockerfile..."
    docker build --quiet -f "$PROJECT_DIR/docker/ppsspp.Dockerfile" \
        -t "$PPSSPP_IMAGE" "$PROJECT_DIR" >/dev/null
fi

FAILURES=0
fail() {
    echo "FAIL: $*"
    FAILURES=$((FAILURES + 1))
}

# run_ppsspp <name> <eboot> <timeout> [--selftest]
# Copies the EBOOT into <workdir>/<name>/game (mounted as host0:), creates a
# fresh memory stick at <workdir>/<name>/ms (ms0:), runs PPSSPPHeadless with
# full logging and keeps only the lines the checks need in <name>/ppsspp.log.
run_ppsspp() {
    local name="$1" eboot="$2" timeout="$3" mode="${4:-}"
    local dir="$WORKDIR/$name"
    rm -rf "$dir"
    mkdir -p "$dir/game" "$dir/ms/PSP/GAME/OASISOS"
    cp "$eboot" "$dir/game/EBOOT.PBP"
    if [ "$mode" = "--selftest" ]; then
        touch "$dir/ms/SELFTEST"
    fi

    echo "--- $name: PPSSPPHeadless (timeout ${timeout}s)"
    # PPSSPPHeadless keeps its memory stick in $HOME/.ppsspp. It exits 0
    # whether the guest finished, crashed or timed out, so the verdict comes
    # from its output, not its exit code. -l is very chatty (millions of
    # lines); stream it through grep instead of storing it.
    local rc
    set +e
    docker run --rm \
        --user "$(id -u):$(id -g)" \
        -e HOME=/smoke/home \
        -v "$dir/game:/smoke/game:ro" \
        -v "$dir/ms:/smoke/home/.ppsspp" \
        --entrypoint PPSSPPHeadless \
        "$PPSSPP_IMAGE" \
        /smoke/game/EBOOT.PBP -r /smoke/game --timeout="$timeout" -l 2>&1 |
        grep -a -E "^TIMEOUT\$|$CRASH_RE|sceNetInetListen\(" >"$dir/ppsspp.log"
    rc="${PIPESTATUS[0]}"
    set -e
    # 125-127: docker itself failed (bad image, mount or entrypoint).
    if [ "$rc" -ge 125 ]; then
        fail "$name: docker run failed with exit code $rc"
    fi
}

check_no_crash() {
    local name="$1"
    if grep -a -q -E "$CRASH_RE" "$WORKDIR/$name/ppsspp.log"; then
        fail "$name: PPSSPP reported a bad access:"
        grep -a -E "$CRASH_RE" "$WORKDIR/$name/ppsspp.log" | head -5
    fi
}

timed_out() {
    grep -a -q -x 'TIMEOUT' "$WORKDIR/$1/ppsspp.log"
}

test_backend_boot() {
    local name=backend-boot
    run_ppsspp "$name" "$BACKEND" "$TIMEOUT"
    local log="$WORKDIR/$name/ms/PSP/GAME/OASISOS/eboot.log"
    timed_out "$name" || fail "$name: EBOOT exited before the ${TIMEOUT}s timeout (crash or quit)"
    check_no_crash "$name"
    if [ ! -f "$log" ]; then
        fail "$name: no ms0:/PSP/GAME/OASISOS/eboot.log written"
        return
    fi
    for marker in '[EBOOT] psp_main entered' '[EBOOT] entering main loop'; do
        grep -a -q -F "$marker" "$log" || fail "$name: eboot.log lacks '$marker'"
    done
    local frames
    frames=$(grep -a -o -E '^\[MAIN\] .*frame=[0-9]+' "$log" | sed -E 's/.*frame=//' |
        sort -n | tail -1 || true)
    if [ "${frames:-0}" -lt "$MIN_FRAME" ]; then
        fail "$name: main loop reached frame ${frames:-0}, expected >= $MIN_FRAME"
    fi
    echo "eboot.log (head):"
    head -20 "$log"
}

test_backend_selftest() {
    local name=backend-selftest
    run_ppsspp "$name" "$BACKEND" "$SELFTEST_TIMEOUT" --selftest
    local log="$WORKDIR/$name/ms/PSP/GAME/OASISOS/selftest.log"
    timed_out "$name" && fail "$name: self-test did not exit within ${SELFTEST_TIMEOUT}s"
    check_no_crash "$name"
    if [ ! -f "$log" ]; then
        fail "$name: no ms0:/PSP/GAME/OASISOS/selftest.log written"
        return
    fi
    cat "$log"
    grep -a -q -x 'EXIT_CODE: 0' "$log" || fail "$name: selftest.log lacks 'EXIT_CODE: 0'"
}

test_recovery_boot() {
    local name=recovery-boot
    run_ppsspp "$name" "$RECOVERY" "$TIMEOUT"
    timed_out "$name" || fail "$name: EBOOT exited before the ${TIMEOUT}s timeout (crash or quit)"
    check_no_crash "$name"
    grep -a -q -F 'sceNetInetListen(' "$WORKDIR/$name/ppsspp.log" ||
        fail "$name: never reached its TCP server (no sceNetInetListen call)"
}

for pair in "backend:$BACKEND" "recovery:$RECOVERY"; do
    path="${pair#*:}"
    if [ -n "$path" ] && [ ! -s "$path" ]; then
        fail "${pair%%:*} EBOOT missing or empty: $path"
    fi
done

if [ -n "$BACKEND" ] && [ -s "$BACKEND" ]; then
    test_backend_boot
    test_backend_selftest
fi
if [ -n "$RECOVERY" ] && [ -s "$RECOVERY" ]; then
    test_recovery_boot
fi

echo ""
echo "Logs: $WORKDIR"
if [ "$FAILURES" -gt 0 ]; then
    echo "PSP smoke test: $FAILURES check(s) failed"
    exit 1
fi
echo "PSP smoke test: passed"
