"""Unit tests for psp_remote.py: screencap decoding and the PSK handshake.

Run: python3 -m unittest discover -s scripts -p "test_*.py"
"""

import hashlib
import hmac
import os
import socket
import struct
import sys
import tempfile
import threading
import unittest
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from psp_remote import (  # noqa: E402
    AuthError,
    PspConnection,
    _write_png_rgb,
    parse_screencap,
    psk_response,
    psm8888_to_rgb,
)


def _psm8888(r, g, b, a=0):
    """One PSM_8888 pixel as the PSP stores it: little-endian 0xAABBGGRR."""
    return struct.pack("<I", (a << 24) | (b << 16) | (g << 8) | r)


class Psm8888ToRgbTest(unittest.TestCase):
    def test_primaries_keep_their_channels(self):
        # The old decoder read the bytes as A,B,G,R: red came out as
        # transparent black and blue as cyan.
        pixels = _psm8888(255, 0, 0) + _psm8888(0, 255, 0) + _psm8888(0, 0, 255)
        rgb = psm8888_to_rgb(pixels, 3, 1)
        self.assertEqual(bytes(rgb), bytes([255, 0, 0, 0, 255, 0, 0, 0, 255]))

    def test_alpha_is_ignored(self):
        # The GE leaves alpha 0 in the framebuffer; the display ignores it.
        for alpha in (0x00, 0x80, 0xFF):
            rgb = psm8888_to_rgb(_psm8888(10, 20, 30, alpha), 1, 1)
            self.assertEqual(bytes(rgb), bytes([10, 20, 30]))

    def test_short_read_pads_with_black(self):
        rgb = psm8888_to_rgb(_psm8888(1, 2, 3) + b"\x09\x09", 2, 1)
        self.assertEqual(bytes(rgb), bytes([1, 2, 3, 0, 0, 0]))


class ParseScreencapTest(unittest.TestCase):
    def test_header_gives_dimensions(self):
        body = _psm8888(1, 2, 3)
        self.assertEqual(parse_screencap(b"1 1\n" + body), (1, 1, body))

    def test_rejects_missing_or_bad_header(self):
        self.assertIsNone(parse_screencap(b""))
        self.assertIsNone(parse_screencap(b"no newline"))
        self.assertIsNone(parse_screencap(b"wide tall\n"))


class WritePngTest(unittest.TestCase):
    def test_writes_opaque_rgb_png(self):
        rgb = bytes([255, 0, 0, 0, 0, 255])
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "cap.png")
            _write_png_rgb(path, 2, 1, rgb)
            with open(path, "rb") as f:
                data = f.read()
        self.assertTrue(data.startswith(b"\x89PNG\r\n\x1a\n"))
        width, height, depth, colour_type = struct.unpack(">IIBB", data[16:26])
        self.assertEqual((width, height, depth, colour_type), (2, 1, 8, 2))
        idat_len = struct.unpack(">I", data[33:37])[0]
        self.assertEqual(data[37:41], b"IDAT")
        raw = zlib.decompress(data[41:41 + idat_len])
        self.assertEqual(raw, b"\x00" + rgb)


NONCE = "0123456789abcdef0123456789abcdef"


def _fake_server(psk):
    """One-shot server speaking the EBOOT's cmd_server protocol.

    Returns `(port, thread, seen)`; `seen` collects the command line the
    server received after a successful handshake.
    """
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    port = listener.getsockname()[1]
    seen = []

    def serve():
        conn, _ = listener.accept()
        with conn, listener:
            f = conn.makefile("rb")
            conn.sendall(f"AUTH_REQUIRED {NONCE}\n".encode())
            answer = f.readline().decode().strip()
            expected = hmac.new(psk.encode(), NONCE.encode(), hashlib.sha256).hexdigest()
            if answer != f"AUTH {expected}":
                conn.sendall(b"AUTH_FAIL\n")
                return
            conn.sendall(b"AUTH_OK\n")
            seen.append(f.readline().decode().strip())
            conn.sendall(b"pong\n")

    thread = threading.Thread(target=serve, daemon=True)
    thread.start()
    return port, thread, seen


class AuthTest(unittest.TestCase):
    def test_psk_response_matches_openssl_hmac(self):
        # printf %s abc | openssl dgst -sha256 -hmac key
        self.assertEqual(
            psk_response("key", "abc"),
            "9c196e32dc0175f86f4b1cb89289d6619de6bee699e4c378e68309ed97a1a6ab",
        )

    def test_command_runs_after_handshake(self):
        port, thread, seen = _fake_server("correct horse")
        conn = PspConnection("127.0.0.1", port, timeout=5, psk="correct horse")
        self.assertEqual(conn.send_command("ping"), "pong")
        thread.join(5)
        self.assertEqual(seen, ["ping"])

    def test_wrong_key_is_rejected(self):
        port, thread, seen = _fake_server("correct horse")
        conn = PspConnection("127.0.0.1", port, timeout=5, psk="battery staple")
        with self.assertRaises(AuthError):
            conn.send_command("ping")
        thread.join(5)
        self.assertEqual(seen, [])

    def test_missing_key_is_an_error(self):
        port, thread, _ = _fake_server("correct horse")
        conn = PspConnection("127.0.0.1", port, timeout=5, psk="")
        with self.assertRaises(AuthError):
            conn.ping()
        thread.join(5)


if __name__ == "__main__":
    unittest.main()
