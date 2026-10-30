#!/usr/bin/env python3
"""Render the GitHub Pages social preview card and touch icon from skin screenshots.

Writes site/img/og-card.png (1200x630, Open Graph / Twitter) and
site/img/apple-touch-icon.png (180x180). Re-run after re-capturing screenshots:

    python3 scripts/site-og-card.py

Needs Pillow. Fonts: the first available of each FONT_* list below (DejaVu on
Linux, Segoe UI / Consolas on Windows); pass --sans/--mono to override.
"""
import argparse
import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SHOTS = os.path.join(ROOT, "screenshots")
OUT = os.path.join(ROOT, "site", "img")

BG = (8, 9, 13)
LINE = (34, 39, 55)
TEXT = (232, 234, 240)
MUTED = (160, 167, 183)
ACCENT = (255, 138, 61)
LIME = (184, 228, 90)
SKY = (124, 199, 255)

FONT_SANS = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "C:/Windows/Fonts/segoeuib.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
]
FONT_MONO = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
    "C:/Windows/Fonts/consolab.ttf",
    "/System/Library/Fonts/Menlo.ttc",
]


def font(paths, size):
    for p in paths:
        if p and os.path.exists(p):
            return ImageFont.truetype(p, size)
    raise SystemExit(f"no usable font among {paths}; pass --sans/--mono")


def shot(skin, view="01_dashboard"):
    return Image.open(os.path.join(SHOTS, skin, f"{view}.png")).convert("RGB")


def window(img, scale, bar=26):
    """Frame a capture like the landing page's OASIS window."""
    w, h = int(img.width * scale), int(img.height * scale)
    resample = Image.NEAREST if img.width <= 640 else Image.LANCZOS
    body = img.resize((w, h), resample)
    pad = 6
    frame = Image.new("RGB", (w + 2 * pad, h + bar + pad), (22, 26, 36))
    d = ImageDraw.Draw(frame)
    d.rectangle([0, 0, frame.width - 1, frame.height - 1], outline=(47, 53, 72))
    for i, c in enumerate([ACCENT, (47, 53, 72), (47, 53, 72)]):
        cx = 16 + i * 16
        d.ellipse([cx - 5, bar // 2 - 5, cx + 5, bar // 2 + 5], fill=c)
    frame.paste(body, (pad, bar))
    return frame


def shadowed(canvas, img, xy, radius=28, alpha=170):
    sh = Image.new("L", (img.width + radius * 4, img.height + radius * 4), 0)
    ImageDraw.Draw(sh).rectangle(
        [radius * 2, radius * 2 + 18, radius * 2 + img.width, radius * 2 + img.height + 18], fill=alpha
    )
    sh = sh.filter(ImageFilter.GaussianBlur(radius))
    canvas.paste((0, 0, 0), (xy[0] - radius * 2, xy[1] - radius * 2), sh)
    canvas.paste(img, xy)


def glow(size, center, radius, color, strength):
    layer = Image.new("RGB", size, BG)
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).ellipse(
        [center[0] - radius, center[1] - radius, center[0] + radius, center[1] + radius],
        fill=strength,
    )
    mask = mask.filter(ImageFilter.GaussianBlur(radius // 2))
    layer.paste(color, (0, 0), mask)
    return layer, mask


def og_card(sans, mono):
    W, H = 1200, 630
    im = Image.new("RGB", (W, H), BG)
    for center, r, col, s in [((980, 180), 330, ACCENT, 70), ((90, 40), 260, SKY, 40),
                              ((1150, 600), 220, LIME, 35)]:
        layer, mask = glow((W, H), center, r, col, s)
        im.paste(layer, (0, 0), mask)
    d = ImageDraw.Draw(im)
    for x in range(0, W, 48):
        d.line([(x, 0), (x, H)], fill=(14, 16, 22))
    for y in range(0, H, 48):
        d.line([(0, y), (W, y)], fill=(14, 16, 22))

    back = window(shot("xp"), 0.44)
    front = window(shot("psix-hifi"), 1.1)
    shadowed(im, back, (W - back.width - 34, 52))
    shadowed(im, front, (W - front.width - 64, H - front.height - 58))

    d = ImageDraw.Draw(im)
    x = 64
    # brand mark + wordmark
    d.rounded_rectangle([x, 70, x + 34, 92], radius=11, fill=ACCENT)
    d.rounded_rectangle([x + 5, 75, x + 29, 87], radius=6, fill=BG)
    f_brand = font(mono, 30)
    d.text((x + 48, 64), "OASIS", font=f_brand, fill=TEXT)
    ox = x + 48 + d.textlength("OASIS", font=f_brand)
    d.text((ox, 64), "_", font=f_brand, fill=ACCENT)
    ox += d.textlength("_", font=f_brand)
    d.text((ox, 64), "OS", font=f_brand, fill=TEXT)

    f_h = font(sans, 66)
    d.text((x, 168), "One OS shell.", font=f_h, fill=TEXT)
    d.text((x, 246), "Any pixel", font=f_h, fill=ACCENT)
    d.text((x, 324), "buffer.", font=f_h, fill=ACCENT)

    f_s = font(mono, 21)
    lines = ["An embeddable OS framework", "in Rust: 17 skins, 15 apps,", "a browser engine and a shell."]
    for i, line in enumerate(lines):
        d.text((x, 430 + i * 30), line, font=f_s, fill=MUTED)

    f_t = font(mono, 17)
    d.line([(x, 545), (x + 430, 545)], fill=LINE, width=1)
    d.text((x, 560), "PSP  ·  SDL3  ·  WASM  ·  UE5", font=f_t, fill=SKY)
    im.save(os.path.join(OUT, "og-card.png"), optimize=True)


def touch_icon():
    S = 180
    im = Image.new("RGB", (S, S), BG)
    d = ImageDraw.Draw(im)
    # same pill mark as the site header / favicon
    d.rounded_rectangle([28, 58, 152, 122], radius=32, fill=ACCENT)
    d.rounded_rectangle([46, 76, 134, 104], radius=14, fill=BG)
    im.save(os.path.join(OUT, "apple-touch-icon.png"), optimize=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--sans")
    ap.add_argument("--mono")
    a = ap.parse_args()
    sans = [a.sans] + FONT_SANS if a.sans else FONT_SANS
    mono = [a.mono] + FONT_MONO if a.mono else FONT_MONO
    os.makedirs(OUT, exist_ok=True)
    og_card(sans, mono)
    touch_icon()
    print("wrote", os.path.join(OUT, "og-card.png"), "and apple-touch-icon.png")


if __name__ == "__main__":
    main()
