"""Unit tests for the screencap decoding in psp_remote.py.

Run: python3 -m unittest discover -s scripts -p "test_*.py"
"""

import os
import struct
import sys
import tempfile
import unittest
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from psp_remote import _write_png_rgb, parse_screencap, psm8888_to_rgb  # noqa: E402


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


if __name__ == "__main__":
    unittest.main()
