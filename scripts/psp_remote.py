"""
PSP Remote Development Toolkit

Unified tool for PSP development over WiFi: deploy, reboot, input injection,
screenshots, log reading, and automated test sequences. Replaces psp-devloop.sh
and psp_demo.py with proper socket handling, retries, and CRC32 verification.

Usage:
    python3 scripts/psp_remote.py ping
    python3 scripts/psp_remote.py status
    python3 scripts/psp_remote.py deploy <eboot>
    python3 scripts/psp_remote.py reboot
    python3 scripts/psp_remote.py hard-reboot
    python3 scripts/psp_remote.py screencap [output.png]
    python3 scripts/psp_remote.py log [--full]
    python3 scripts/psp_remote.py press <button>
    python3 scripts/psp_remote.py hold <button> <ms>
    python3 scripts/psp_remote.py cursor <x> <y>
    python3 scripts/psp_remote.py cycle <eboot>
    python3 scripts/psp_remote.py build-cycle
    python3 scripts/psp_remote.py skins
    python3 scripts/psp_remote.py skin <name>
    python3 scripts/psp_remote.py upload <local> <remote>
    python3 scripts/psp_remote.py sequence <file.yaml>

Environment:
    PSP_IP   (default: 192.168.0.249)
    PSP_PORT (default: 9293)
"""

import hashlib
import os
import socket
import struct
import subprocess
import sys
import time
import zlib

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

DEFAULT_IP = "192.168.0.249"
DEFAULT_PORT = 9293
DEFAULT_TIMEOUT = 5
DEPLOY_TIMEOUT = 60
RECV_BUF = 4096
MAX_RETRIES = 3
RETRY_BACKOFF = 2  # seconds

EBOOT_PATH = os.path.join(
    os.path.dirname(__file__), "..",
    "crates", "oasis-backend-psp", "target",
    "mipsel-sony-psp-std", "release", "EBOOT.PBP",
)

# ---------------------------------------------------------------------------
# PspConnection — low-level TCP communication
# ---------------------------------------------------------------------------

class PspConnection:
    """Manages a TCP connection to the PSP command server."""

    def __init__(self, ip=None, port=None, timeout=DEFAULT_TIMEOUT):
        self.ip = ip or os.environ.get("PSP_IP", DEFAULT_IP)
        self.port = int(port or os.environ.get("PSP_PORT", DEFAULT_PORT))
        self.timeout = timeout
        self._sock = None

    def connect(self):
        if self._sock:
            return
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(self.timeout)
        s.connect((self.ip, self.port))
        self._sock = s

    def close(self):
        if self._sock:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None

    def reconnect(self):
        self.close()
        self.connect()

    def _ensure_connected(self):
        if not self._sock:
            self.connect()

    def send_command(self, cmd):
        """Send a single-line command, return the text response."""
        self._ensure_connected()
        try:
            self._sock.sendall((cmd + "\n").encode())
            chunks = []
            while True:
                try:
                    data = self._sock.recv(RECV_BUF)
                    if not data:
                        break
                    chunks.append(data)
                except socket.timeout:
                    break
            return b"".join(chunks).decode(errors="replace").strip()
        finally:
            # PSP server closes connection after each command, so reset.
            self.close()

    def send_command_raw(self, cmd):
        """Send a command and return raw bytes response."""
        self._ensure_connected()
        try:
            self._sock.sendall((cmd + "\n").encode())
            chunks = []
            while True:
                try:
                    data = self._sock.recv(RECV_BUF)
                    if not data:
                        break
                    chunks.append(data)
                except socket.timeout:
                    break
            return b"".join(chunks)
        finally:
            self.close()

    def ping(self):
        """Returns True if PSP responds to ping."""
        try:
            return self.send_command("ping") == "pong"
        except (OSError, socket.timeout):
            return False

    def wait_online(self, timeout=60, poll_interval=3):
        """Block until PSP responds to ping or timeout expires."""
        deadline = time.monotonic() + timeout
        attempt = 0
        while time.monotonic() < deadline:
            attempt += 1
            if self.ping():
                return True
            remaining = deadline - time.monotonic()
            if remaining > 0:
                time.sleep(min(poll_interval, remaining))
        return False

    def deploy(self, eboot_path, verify_crc=True):
        """Deploy an EBOOT.PBP to the PSP with optional CRC32 verification."""
        with open(eboot_path, "rb") as f:
            data = f.read()

        size = len(data)
        crc = zlib.crc32(data) & 0xFFFFFFFF
        crc_hex = f"{crc:08x}"

        if verify_crc:
            header = f"deploy {size} {crc_hex}\n"
        else:
            header = f"deploy {size}\n"

        self._ensure_connected()
        self._sock.settimeout(DEPLOY_TIMEOUT)
        try:
            self._sock.sendall(header.encode())

            # Send data in chunks with progress.
            sent = 0
            chunk_size = 4096
            while sent < size:
                end = min(sent + chunk_size, size)
                self._sock.sendall(data[sent:end])
                sent = end
                pct = sent * 100 // size
                print(f"\r  Deploying: {sent:,}/{size:,} bytes ({pct}%)", end="", flush=True)
            print()

            # Read response.
            resp = b""
            while True:
                try:
                    chunk = self._sock.recv(RECV_BUF)
                    if not chunk:
                        break
                    resp += chunk
                except socket.timeout:
                    break
            return resp.decode(errors="replace").strip()
        finally:
            self.close()

    def upload(self, local_path, remote_path):
        """Upload a file to an arbitrary ms0: path."""
        with open(local_path, "rb") as f:
            data = f.read()

        size = len(data)
        header = f"upload {size} {remote_path}\n"

        self._ensure_connected()
        self._sock.settimeout(DEPLOY_TIMEOUT)
        try:
            self._sock.sendall(header.encode())
            self._sock.sendall(data)

            resp = b""
            while True:
                try:
                    chunk = self._sock.recv(RECV_BUF)
                    if not chunk:
                        break
                    resp += chunk
                except socket.timeout:
                    break
            return resp.decode(errors="replace").strip()
        finally:
            self.close()

    def screencap(self):
        """Capture screen and return raw ABGR pixel data (480x272)."""
        raw = self.send_command_raw("screencap")
        # Header is "480 272\n", skip it.
        nl = raw.find(b"\n")
        if nl < 0:
            return None
        return raw[nl + 1:]

    def screencap_png(self, output_path):
        """Capture screen and save as PNG."""
        pixels = self.screencap()
        if not pixels:
            print("Error: no screencap data received")
            return False

        expected = 480 * 272 * 4
        if len(pixels) < expected:
            print(f"Warning: got {len(pixels)} bytes, expected {expected}")

        # Convert ABGR to RGBA for PNG.
        rgba = bytearray(expected)
        limit = min(len(pixels), expected)
        limit -= limit % 4
        for i in range(0, limit, 4):
            a, b, g, r = pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]
            rgba[i] = r
            rgba[i + 1] = g
            rgba[i + 2] = b
            rgba[i + 3] = a

        # Write PNG using zlib (no PIL dependency).
        _write_png(output_path, 480, 272, rgba)
        return True


# ---------------------------------------------------------------------------
# PspDevKit — high-level workflows combining connection + actuator
# ---------------------------------------------------------------------------

class PspDevKit:
    """High-level development workflows for PSP."""

    def __init__(self, ip=None, port=None):
        self.conn = PspConnection(ip, port)
        self._actuator = None

    @property
    def actuator(self):
        if self._actuator is None:
            # Import from sibling module.
            sys.path.insert(0, os.path.dirname(__file__))
            from psp_actuator import ActuatorController
            self._actuator = ActuatorController()
        return self._actuator

    def deploy_and_reboot(self, eboot_path):
        """Deploy EBOOT, software reboot, wait for PSP to come back."""
        print(f"Deploying {os.path.basename(eboot_path)}...")
        resp = self.conn.deploy(eboot_path)
        print(f"  Server: {resp}")

        if not resp.startswith("ok"):
            print("Deploy failed, aborting.")
            return False

        print("Rebooting...")
        # New connection for reboot command.
        self.conn.send_command("reboot")

        print("Waiting for PSP to come back online...")
        time.sleep(5)  # Give it time to start rebooting.
        if self.conn.wait_online(timeout=60, poll_interval=3):
            print("PSP is back online.")
            return True
        else:
            print("Timeout waiting for PSP to reconnect.")
            return False

    def hard_reboot_and_wait(self):
        """Power cycle via actuator and wait for PSP to come back."""
        print("Hard rebooting via actuator...")
        self.actuator.hard_reboot()
        print("Waiting for PSP to come back online...")
        if self.conn.wait_online(timeout=90, poll_interval=3):
            print("PSP is back online.")
            return True
        else:
            print("Timeout waiting for PSP after hard reboot.")
            return False

    def build_and_deploy(self):
        """Build PSP EBOOT and deploy it."""
        crate_dir = os.path.join(
            os.path.dirname(__file__), "..",
            "crates", "oasis-backend-psp",
        )
        crate_dir = os.path.abspath(crate_dir)
        eboot = os.path.join(
            crate_dir, "target", "mipsel-sony-psp-std", "release", "EBOOT.PBP",
        )

        print("Building PSP EBOOT...")
        env = os.environ.copy()
        env["RUST_PSP_BUILD_STD"] = "1"
        result = subprocess.run(
            ["cargo", "+nightly", "psp", "--release"],
            cwd=crate_dir,
            env=env,
        )
        if result.returncode != 0:
            print("Build failed.")
            return False

        if not os.path.exists(eboot):
            print(f"EBOOT not found at {eboot}")
            return False

        return self.deploy_and_reboot(eboot)

    def run_sequence(self, steps):
        """Execute a list of command steps.

        Each step is a dict with one key (the command) and a value (the argument).
        Example: [{"deploy": "path/to/EBOOT.PBP"}, {"wait": 30}, {"press": "cross"}]
        """
        for i, step in enumerate(steps):
            if isinstance(step, str):
                step = {step: None}

            for cmd, arg in step.items():
                print(f"[{i + 1}/{len(steps)}] {cmd}: {arg or ''}")

                if cmd == "deploy":
                    resp = self.conn.deploy(str(arg))
                    print(f"  {resp}")
                elif cmd == "reboot":
                    self.conn.send_command("reboot")
                elif cmd == "hard-reboot":
                    self.actuator.hard_reboot()
                elif cmd == "wait":
                    secs = int(arg) if arg else 10
                    for remaining in range(secs, 0, -1):
                        print(f"\r  Waiting: {remaining}s ", end="", flush=True)
                        time.sleep(1)
                    print()
                elif cmd == "wait-online":
                    timeout = int(arg) if arg else 60
                    if not self.conn.wait_online(timeout=timeout):
                        print("  Timeout!")
                        return False
                    print("  Online.")
                elif cmd == "ping":
                    ok = self.conn.ping()
                    print(f"  {'pong' if ok else 'no response'}")
                elif cmd == "press":
                    print(f"  {self.conn.send_command(f'press {arg}')}")
                elif cmd == "hold":
                    parts = str(arg).split()
                    print(f"  {self.conn.send_command(f'hold {parts[0]} {parts[1]}')}")
                elif cmd == "cursor":
                    parts = str(arg).split()
                    print(f"  {self.conn.send_command(f'cursor {parts[0]} {parts[1]}')}")
                elif cmd == "screencap":
                    path = str(arg) if arg else "/tmp/psp_screen.png"
                    if self.conn.screencap_png(path):
                        print(f"  Saved to {path}")
                elif cmd == "screenshot":
                    print(f"  {self.conn.send_command('screenshot')}")
                elif cmd == "status":
                    print(f"  {self.conn.send_command('status')}")
                elif cmd == "log":
                    print(self.conn.send_command("log"))
                elif cmd == "logfull":
                    print(self.conn.send_command("logfull"))
                elif cmd == "upload":
                    parts = str(arg).split(None, 1)
                    print(f"  {self.conn.upload(parts[0], parts[1])}")
                else:
                    # Pass through as raw command.
                    print(f"  {self.conn.send_command(cmd if not arg else f'{cmd} {arg}')}")
        return True


# ---------------------------------------------------------------------------
# Minimal PNG writer (no PIL dependency)
# ---------------------------------------------------------------------------

def _write_png(path, width, height, rgba_data):
    """Write RGBA data as a PNG file using only zlib."""

    def _chunk(chunk_type, data):
        c = chunk_type + data
        crc = zlib.crc32(c) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + c + struct.pack(">I", crc)

    signature = b"\x89PNG\r\n\x1a\n"
    ihdr_data = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    ihdr = _chunk(b"IHDR", ihdr_data)

    # Build raw image data with filter byte 0 (None) per row.
    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)  # filter byte
        row_start = y * stride
        raw.extend(rgba_data[row_start:row_start + stride])

    compressed = zlib.compress(bytes(raw), 9)
    idat = _chunk(b"IDAT", compressed)
    iend = _chunk(b"IEND", b"")

    with open(path, "wb") as f:
        f.write(signature + ihdr + idat + iend)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def _with_retry(fn, retries=MAX_RETRIES, backoff=RETRY_BACKOFF):
    """Retry a function on connection failure."""
    last_err = None
    for attempt in range(retries):
        try:
            return fn()
        except (OSError, socket.timeout) as e:
            last_err = e
            if attempt < retries - 1:
                wait = backoff * (attempt + 1)
                print(f"  Connection failed ({e}), retrying in {wait}s...")
                time.sleep(wait)
    print(f"  Failed after {retries} attempts: {last_err}")
    sys.exit(1)


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help", "help"):
        print(__doc__.strip())
        sys.exit(0)

    cmd = args[0]
    kit = PspDevKit()

    if cmd == "ping":
        ok = _with_retry(lambda: kit.conn.ping())
        print("pong" if ok else "no response")
        sys.exit(0 if ok else 1)

    elif cmd == "status":
        print(_with_retry(lambda: kit.conn.send_command("status")))

    elif cmd == "deploy":
        if len(args) < 2:
            print("Usage: psp_remote.py deploy <eboot>")
            sys.exit(1)
        resp = _with_retry(lambda: kit.conn.deploy(args[1]))
        print(f"Server: {resp}")

    elif cmd == "reboot":
        print(_with_retry(lambda: kit.conn.send_command("reboot")))

    elif cmd == "hard-reboot":
        kit.hard_reboot_and_wait()

    elif cmd == "screencap":
        output = args[1] if len(args) > 1 else "/tmp/psp_screen.png"
        if _with_retry(lambda: kit.conn.screencap_png(output)):
            print(f"Saved to {output}")

    elif cmd == "log":
        full = "--full" in args
        print(_with_retry(lambda: kit.conn.send_command("logfull" if full else "log")))

    elif cmd == "press":
        if len(args) < 2:
            print("Usage: psp_remote.py press <button>")
            sys.exit(1)
        print(_with_retry(lambda: kit.conn.send_command(f"press {args[1]}")))

    elif cmd == "hold":
        if len(args) < 3:
            print("Usage: psp_remote.py hold <button> <ms>")
            sys.exit(1)
        print(_with_retry(lambda: kit.conn.send_command(f"hold {args[1]} {args[2]}")))

    elif cmd == "cursor":
        if len(args) < 3:
            print("Usage: psp_remote.py cursor <x> <y>")
            sys.exit(1)
        print(_with_retry(lambda: kit.conn.send_command(f"cursor {args[1]} {args[2]}")))

    elif cmd == "cycle":
        if len(args) < 2:
            print("Usage: psp_remote.py cycle <eboot>")
            sys.exit(1)
        kit.deploy_and_reboot(args[1])

    elif cmd == "build-cycle":
        kit.build_and_deploy()

    elif cmd == "upload":
        if len(args) < 3:
            print("Usage: psp_remote.py upload <local_path> <remote_path>")
            sys.exit(1)
        resp = _with_retry(lambda: kit.conn.upload(args[1], args[2]))
        print(f"Server: {resp}")

    elif cmd == "sequence":
        if len(args) < 2:
            print("Usage: psp_remote.py sequence <file.yaml>")
            sys.exit(1)
        import yaml
        with open(args[1]) as f:
            steps = yaml.safe_load(f)
        if not kit.run_sequence(steps):
            sys.exit(1)

    elif cmd == "wait-online":
        timeout = int(args[1]) if len(args) > 1 else 60
        if kit.conn.wait_online(timeout=timeout):
            print("PSP is online.")
        else:
            print("Timeout.")
            sys.exit(1)

    else:
        # Pass through as raw command.
        full_cmd = " ".join(args)
        print(_with_retry(lambda: kit.conn.send_command(full_cmd)))


if __name__ == "__main__":
    main()
